//! O `S3` contra um MinIO de verdade (`testes/s3/rodar.sh`, em Docker). Fora do
//! `cargo test --workspace`: o teste é `#[ignore]` e só roda com `TOMNI_UI_S3_TESTE` apontando
//! para o endpoint (`http://127.0.0.1:19000`), com o usuário e a senha `minioadmin` e o bucket
//! `teste`.
#![cfg(all(unix, feature = "s3"))]

mod comum;

use std::io::{Read, Write};
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui_files::s3::{S3, S3Config, S3UrlStyle};
use tomni_ui_files::{EntryKind, Fs};

use comum::dados;

const MIB: usize = 1 << 20;

fn config(bucket: Option<&str>) -> S3Config {
    let endpoint = std::env::var("TOMNI_UI_S3_TESTE")
        .expect("TOMNI_UI_S3_TESTE com o endpoint do MinIO (veja testes/s3/rodar.sh)");
    S3Config {
        bucket: bucket.map(str::to_owned),
        url_style: S3UrlStyle::Path,
        part_size: 5 * MIB as u64,
        ..S3Config::new(endpoint, "minioadmin", "minioadmin")
    }
}

fn ler_tudo(fs: &mut dyn Fs, caminho: &str, offset: u64) -> Vec<u8> {
    let mut arquivo = fs.open_read(caminho, offset).unwrap();
    let mut saida = Vec::new();
    arquivo.read_to_end(&mut saida).unwrap();
    arquivo.finish().unwrap();
    saida
}

fn nomes(fs: &mut dyn Fs, caminho: &str) -> Vec<String> {
    let mut nomes: Vec<String> = fs
        .list(caminho)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    nomes.sort();
    nomes
}

#[test]
#[ignore = "precisa do MinIO (testes/s3/rodar.sh)"]
fn o_s3_contra_o_minio_de_verdade() {
    let base = format!("m32-{}", std::process::id());
    let mut s3 = S3::connect(config(Some("teste"))).expect("conectar ao bucket teste");

    // Sem bucket, a raiz lista os buckets.
    let mut todos = S3::connect(config(None)).expect("conectar sem bucket");
    assert!(nomes(&mut todos, "/").contains(&"teste".to_owned()));
    assert_eq!(
        nomes(&mut todos, &format!("/teste/{base}")),
        Vec::<String>::new()
    );

    // Pequeno, com a data, e grande em partes.
    let data = UNIX_EPOCH + Duration::new(1_600_000_000, 0);
    let pequeno = format!("/{base}/pasta/pequeno.txt");
    let mut envio = s3.open_write_with(&pequeno, 0, Some(data)).unwrap();
    envio.write_all(b"ola").unwrap();
    envio.finish().unwrap();
    let grande = dados(12 * MIB + 5);
    let caminho_grande = format!("/{base}/grande.bin");
    let mut envio = s3.open_write(&caminho_grande, 0).unwrap();
    envio.write_all(&grande).unwrap();
    envio.finish().unwrap();

    assert_eq!(nomes(&mut s3, &format!("/{base}")), ["grande.bin", "pasta"]);
    assert_eq!(ler_tudo(&mut s3, &pequeno, 0), b"ola");
    assert_eq!(ler_tudo(&mut s3, &caminho_grande, 0), grande);
    assert_eq!(
        ler_tudo(&mut s3, &caminho_grande, 7 * MIB as u64),
        grande[7 * MIB..]
    );
    let entrada = s3.stat(&pequeno).unwrap();
    assert_eq!(entrada.kind, EntryKind::File);
    assert_eq!(entrada.modified, Some(data));
    assert!(s3.stat(&format!("/{base}/pasta")).unwrap().is_dir());

    // Retomar: duas partes ficam, outra conexão continua.
    let retomado = dados(15 * MIB);
    let caminho_retomado = format!("/{base}/retomado.bin");
    {
        let mut outra = S3::connect(config(Some("teste"))).unwrap();
        let mut envio = outra.open_write(&caminho_retomado, 0).unwrap();
        envio.write_all(&retomado[..10 * MIB]).unwrap();
    }
    assert_eq!(
        s3.resume_offset(&caminho_retomado).unwrap(),
        10 * MIB as u64
    );
    let pendentes = s3.pending_uploads(&format!("/{base}")).unwrap();
    assert_eq!(pendentes.len(), 1, "{pendentes:?}");
    let mut envio = s3.open_write(&caminho_retomado, 10 * MIB as u64).unwrap();
    envio.write_all(&retomado[10 * MIB..]).unwrap();
    envio.finish().unwrap();
    assert_eq!(ler_tudo(&mut s3, &caminho_retomado, 0), retomado);

    // Um envio abandonado e abortado.
    let abandonado = format!("/{base}/abandonado.bin");
    {
        let mut outra = S3::connect(config(Some("teste"))).unwrap();
        let mut envio = outra.open_write(&abandonado, 0).unwrap();
        envio.write_all(&dados(5 * MIB)).unwrap();
    }
    let pendentes = s3.pending_uploads(&format!("/{base}")).unwrap();
    assert_eq!(pendentes.len(), 1, "{pendentes:?}");
    s3.abort_upload(&pendentes[0].path, &pendentes[0].upload_id)
        .unwrap();
    assert!(s3.pending_uploads(&format!("/{base}")).unwrap().is_empty());

    // Renomear (arquivo e pasta), classe, link pré-assinado.
    s3.rename(&pequeno, &format!("/{base}/pasta/outro.txt"), false)
        .unwrap();
    s3.rename(&format!("/{base}/pasta"), &format!("/{base}/nova"), false)
        .unwrap();
    assert_eq!(nomes(&mut s3, &format!("/{base}/nova")), ["outro.txt"]);
    let outro = format!("/{base}/nova/outro.txt");
    s3.set_storage_class(&outro, "REDUCED_REDUNDANCY").unwrap();
    assert_eq!(s3.head(&outro).unwrap().storage_class, "REDUCED_REDUNDANCY");
    let link = s3.presign_get(&outro, Duration::from_secs(60)).unwrap();
    let baixado = ureq::get(&link)
        .call()
        .unwrap()
        .into_body()
        .read_to_string()
        .unwrap();
    assert_eq!(baixado, "ola");

    // Apagar a pasta toda.
    s3.remove_dir(&format!("/{base}")).unwrap();
    assert!(s3.list(&format!("/{base}")).unwrap().is_empty());

    // O segredo errado.
    let erro = S3::connect(S3Config {
        secret_key: "errado".into(),
        ..config(Some("teste"))
    })
    .unwrap_err();
    assert!(erro.to_string().contains("assinatura"), "{erro}");
}
