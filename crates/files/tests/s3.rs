//! O `S3` contra o servidor de teste (`comum/s3d.rs`), que refaz a assinatura SigV4 como o
//! MinIO e a AWS, sem rede de verdade.
#![cfg(all(unix, feature = "s3"))]

mod comum;

use std::io::{self, Read, Write};
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui_files::s3::{S3, S3Config, TlsMode};
use tomni_ui_files::{Conflict, Endpoint, EntryKind, Fs, TransferState, Transfers};

use comum::s3d::{ACESSO, Opcoes, S3d, SEGREDO, certificado_pem};
use comum::{Pasta, dados};

const MIB: usize = 1 << 20;

fn servidor() -> S3d {
    let servidor = S3d::novo();
    servidor.criar_bucket("bkt");
    servidor
}

fn conectar(servidor: &S3d) -> S3 {
    S3::connect(servidor.config(Some("bkt"))).expect("conectar")
}

fn ler_tudo(fs: &mut dyn Fs, caminho: &str, offset: u64) -> Vec<u8> {
    let mut arquivo = fs.open_read(caminho, offset).unwrap();
    let mut saida = Vec::new();
    arquivo.read_to_end(&mut saida).unwrap();
    arquivo.finish().unwrap();
    saida
}

fn gravar(fs: &mut dyn Fs, caminho: &str, conteudo: &[u8]) {
    let mut arquivo = fs.open_write(caminho, 0).unwrap();
    arquivo.write_all(conteudo).unwrap();
    arquivo.finish().unwrap();
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

fn com_partes_de_5_mib(servidor: &S3d) -> S3Config {
    S3Config {
        part_size: 5 * MIB as u64,
        ..servidor.config(Some("bkt"))
    }
}

// ----- listar -----

#[test]
fn lista_arquivos_pastas_e_esconde_o_marcador_de_pasta() {
    let servidor = servidor();
    servidor.colocar("bkt", "a.txt", b"a");
    servidor.colocar("bkt", "docs/b.txt", b"bb");
    servidor.colocar("bkt", "docs/sub/c.txt", b"ccc");
    servidor.colocar("bkt", "vazia/", b"");
    let mut s3 = conectar(&servidor);

    let raiz = s3.list("/").unwrap();
    let kinds: Vec<(String, EntryKind)> = {
        let mut v: Vec<_> = raiz.iter().map(|e| (e.name.clone(), e.kind)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(
        kinds,
        [
            ("a.txt".to_owned(), EntryKind::File),
            ("docs".to_owned(), EntryKind::Dir),
            ("vazia".to_owned(), EntryKind::Dir),
        ]
    );
    assert_eq!(raiz.iter().find(|e| e.name == "a.txt").unwrap().size, 1);
    assert!(
        raiz.iter()
            .find(|e| e.name == "a.txt")
            .unwrap()
            .modified
            .is_some()
    );
    assert_eq!(nomes(&mut s3, "/docs"), ["b.txt", "sub"]);
    assert_eq!(nomes(&mut s3, "/docs/sub"), ["c.txt"]);
    // O marcador `vazia/` não é um arquivo da pasta.
    assert!(s3.list("/vazia").unwrap().is_empty());
}

#[test]
fn sem_bucket_a_raiz_lista_os_buckets() {
    let servidor = servidor();
    servidor.criar_bucket("outro");
    servidor.colocar("outro", "x/y.txt", b"y");
    let mut s3 = S3::connect(servidor.config(None)).unwrap();

    assert_eq!(nomes(&mut s3, "/"), ["bkt", "outro"]);
    assert_eq!(nomes(&mut s3, "/outro"), ["x"]);
    assert_eq!(nomes(&mut s3, "/outro/x"), ["y.txt"]);
    assert_eq!(ler_tudo(&mut s3, "/outro/x/y.txt", 0), b"y");
    assert!(s3.stat("/outro").unwrap().is_dir());
    assert_eq!(s3.stat("/outro/x/y.txt").unwrap().size, 1);
}

#[test]
fn pagina_dois_mil_e_quinhentos_objetos() {
    let servidor = servidor();
    for i in 0..2500 {
        servidor.colocar("bkt", &format!("lote/{i:04}.txt"), b"x");
    }
    let mut s3 = conectar(&servidor);
    servidor.limpar_pedidos();

    assert_eq!(s3.list("/lote").unwrap().len(), 2500);
    assert_eq!(servidor.contar("GET", "list-type=2"), 3);
}

#[test]
fn nomes_com_espaco_acento_e_mais_sobrevivem_a_listagem() {
    let servidor = servidor();
    servidor.colocar("bkt", "minha pasta/açaí +1 (cópia).txt", b"ok");
    let mut s3 = conectar(&servidor);

    assert_eq!(nomes(&mut s3, "/"), ["minha pasta"]);
    assert_eq!(nomes(&mut s3, "/minha pasta"), ["açaí +1 (cópia).txt"]);
    assert_eq!(
        ler_tudo(&mut s3, "/minha pasta/açaí +1 (cópia).txt", 0),
        b"ok"
    );
}

// ----- stat e leitura -----

#[test]
fn stat_de_arquivo_pasta_e_inexistente() {
    let servidor = servidor();
    servidor.colocar("bkt", "docs/b.txt", b"123");
    let mut s3 = conectar(&servidor);

    let arquivo = s3.stat("/docs/b.txt").unwrap();
    assert_eq!(arquivo.kind, EntryKind::File);
    assert_eq!(arquivo.size, 3);
    assert_eq!(arquivo.name, "b.txt");
    assert_eq!(s3.stat("/docs").unwrap().kind, EntryKind::Dir);
    assert_eq!(s3.stat("/").unwrap().kind, EntryKind::Dir);
    let erro = s3.stat("/nao/existe").unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::NotFound, "{erro}");
    // O `lstat` é o `stat` (não há links).
    assert_eq!(s3.lstat("/docs/b.txt").unwrap().size, 3);
}

#[test]
fn le_a_partir_de_um_offset() {
    let servidor = servidor();
    let conteudo = dados(100_000);
    servidor.colocar("bkt", "d.bin", &conteudo);
    let mut s3 = conectar(&servidor);

    assert_eq!(ler_tudo(&mut s3, "/d.bin", 0), conteudo);
    assert_eq!(ler_tudo(&mut s3, "/d.bin", 40_000), conteudo[40_000..]);
}

#[test]
fn o_offset_funciona_mesmo_se_o_servidor_ignora_o_range() {
    let servidor = S3d::com_opcoes(Opcoes {
        ignorar_range: true,
        ..Opcoes::default()
    });
    servidor.criar_bucket("bkt");
    let conteudo = dados(50_000);
    servidor.colocar("bkt", "d.bin", &conteudo);
    let mut s3 = conectar(&servidor);

    assert_eq!(ler_tudo(&mut s3, "/d.bin", 12_345), conteudo[12_345..]);
}

#[test]
fn ler_o_que_nao_existe_diz_que_o_objeto_nao_existe() {
    let servidor = servidor();
    let mut s3 = conectar(&servidor);

    let Err(erro) = s3.open_read("/nada.txt", 0) else {
        panic!("deveria falhar");
    };
    assert_eq!(erro.kind(), io::ErrorKind::NotFound);
    assert!(erro.to_string().contains("/nada.txt"), "{erro}");
}

// ----- escrever -----

#[test]
fn grava_um_arquivo_pequeno_com_o_tipo_pela_extensao() {
    let servidor = servidor();
    let mut s3 = conectar(&servidor);

    gravar(&mut s3, "/notas.txt", b"ola");
    gravar(&mut s3, "/dir/sem-extensao", b"x");

    assert_eq!(servidor.objeto("bkt", "notas.txt").unwrap(), b"ola");
    assert_eq!(servidor.tipo("bkt", "notas.txt").unwrap(), "text/plain");
    assert_eq!(
        servidor.tipo("bkt", "dir/sem-extensao").unwrap(),
        "application/octet-stream"
    );
    // Um envio pequeno é um `PutObject` só, sem partes.
    assert_eq!(servidor.contar("POST", "uploads"), 0);
    assert_eq!(servidor.envios_abertos(), 0);
}

#[test]
fn grava_um_arquivo_grande_em_partes() {
    let servidor = servidor();
    let mut s3 = S3::connect(com_partes_de_5_mib(&servidor)).unwrap();
    let conteudo = dados(12 * MIB + 77);

    gravar(&mut s3, "/grande.bin", &conteudo);

    assert_eq!(servidor.objeto("bkt", "grande.bin").unwrap(), conteudo);
    assert_eq!(servidor.contar("PUT", "partNumber"), 3);
    assert_eq!(servidor.envios_abertos(), 0);
}

#[test]
fn retoma_um_envio_de_20_mib_depois_da_segunda_parte() {
    let servidor = servidor();
    let conteudo = dados(20 * MIB);
    {
        let mut s3 = S3::connect(com_partes_de_5_mib(&servidor)).unwrap();
        assert!(s3.caps().resume_upload);
        let mut envio = s3.open_write("/retoma.bin", 0).unwrap();
        for bloco in conteudo[..10 * MIB].chunks(MIB) {
            envio.write_all(bloco).unwrap();
        }
        // Soltar sem `finish` é cancelar: as partes ficam no servidor.
    }
    assert!(servidor.objeto("bkt", "retoma.bin").is_none());
    assert_eq!(servidor.envios_abertos(), 1);

    // Outra conexão (o app reabriu) descobre onde parou.
    let mut s3 = S3::connect(com_partes_de_5_mib(&servidor)).unwrap();
    assert_eq!(s3.resume_offset("/retoma.bin").unwrap(), (10 * MIB) as u64);
    assert_eq!(s3.resume_offset("/outro.bin").unwrap(), 0);
    let mut envio = s3.open_write("/retoma.bin", (10 * MIB) as u64).unwrap();
    for bloco in conteudo[10 * MIB..].chunks(MIB) {
        envio.write_all(bloco).unwrap();
    }
    envio.finish().unwrap();

    assert_eq!(servidor.objeto("bkt", "retoma.bin").unwrap(), conteudo);
    assert_eq!(servidor.envios_abertos(), 0);
}

#[test]
fn retomar_sem_um_envio_que_bata_com_o_tamanho_falha() {
    let servidor = servidor();
    let mut s3 = S3::connect(com_partes_de_5_mib(&servidor)).unwrap();

    let Err(erro) = s3.open_write("/nada.bin", 5 * MIB as u64) else {
        panic!("deveria falhar");
    };
    assert_eq!(erro.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn a_data_do_arquivo_de_origem_volta_como_modified() {
    let servidor = servidor();
    let mut s3 = conectar(&servidor);
    let data = UNIX_EPOCH + Duration::new(1_600_000_000, 500_000_000);

    let mut envio = s3.open_write_with("/datado.txt", 0, Some(data)).unwrap();
    envio.write_all(b"x").unwrap();
    envio.finish().unwrap();

    assert!(servidor.metadado("bkt", "datado.txt", "mtime").is_some());
    let lido = s3.stat("/datado.txt").unwrap().modified.unwrap();
    let diferenca = lido
        .duration_since(data)
        .unwrap_or_else(|erro| erro.duration());
    assert!(diferenca < Duration::from_millis(1), "{diferenca:?}");
    // Sem data, o `modified` é a do servidor.
    gravar(&mut s3, "/sem-data.txt", b"y");
    assert!(s3.stat("/sem-data.txt").unwrap().modified.is_some());
    assert!(s3.head("/sem-data.txt").unwrap().metadata.is_empty());
}

// ----- mudar o que existe -----

#[test]
fn renomeia_um_arquivo_e_uma_pasta() {
    let servidor = servidor();
    servidor.colocar("bkt", "a.txt", b"a");
    servidor.colocar("bkt", "pasta/x.txt", b"x");
    servidor.colocar("bkt", "pasta/sub/y.txt", b"y");
    let mut s3 = conectar(&servidor);

    s3.rename("/a.txt", "/b.txt", false).unwrap();
    s3.rename("/pasta", "/nova", false).unwrap();

    assert_eq!(
        servidor.chaves("bkt"),
        ["b.txt", "nova/sub/y.txt", "nova/x.txt"]
    );
    assert_eq!(servidor.objeto("bkt", "nova/sub/y.txt").unwrap(), b"y");
}

#[test]
fn renomear_sobre_um_existente_respeita_o_overwrite() {
    let servidor = servidor();
    servidor.colocar("bkt", "a.txt", b"a");
    servidor.colocar("bkt", "b.txt", b"b");
    let mut s3 = conectar(&servidor);

    let erro = s3.rename("/a.txt", "/b.txt", false).unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::AlreadyExists);
    s3.rename("/a.txt", "/b.txt", true).unwrap();
    assert_eq!(servidor.chaves("bkt"), ["b.txt"]);
    assert_eq!(servidor.objeto("bkt", "b.txt").unwrap(), b"a");
}

#[test]
fn a_copia_acima_do_limite_vai_em_partes() {
    let servidor = servidor();
    let conteudo = dados(12 * MIB);
    servidor.colocar("bkt", "grande.bin", &conteudo);
    let mut s3 = S3::connect(S3Config {
        copy_limit: 5 * MIB as u64,
        ..servidor.config(Some("bkt"))
    })
    .unwrap();

    s3.rename("/grande.bin", "/copia.bin", false).unwrap();

    assert_eq!(servidor.chaves("bkt"), ["copia.bin"]);
    assert_eq!(servidor.objeto("bkt", "copia.bin").unwrap(), conteudo);
    assert_eq!(servidor.contar("PUT", "partNumber"), 3);
}

#[test]
fn apaga_uma_pasta_com_1200_objetos_em_dois_lotes() {
    let servidor = servidor();
    for i in 0..1200 {
        servidor.colocar("bkt", &format!("lixo/{i:04}.txt"), b"x");
    }
    servidor.colocar("bkt", "fica.txt", b"f");
    let mut s3 = conectar(&servidor);
    servidor.limpar_pedidos();

    s3.remove_dir("/lixo").unwrap();

    assert_eq!(servidor.chaves("bkt"), ["fica.txt"]);
    assert_eq!(servidor.contar("POST", "delete"), 2);
}

#[test]
fn apaga_um_arquivo_e_cria_uma_pasta() {
    let servidor = servidor();
    servidor.colocar("bkt", "a.txt", b"a");
    let mut s3 = conectar(&servidor);

    s3.mkdir("/nova").unwrap();
    assert_eq!(servidor.chaves("bkt"), ["a.txt", "nova/"]);
    assert!(s3.stat("/nova").unwrap().is_dir());
    s3.remove_file("/a.txt").unwrap();
    assert_eq!(servidor.chaves("bkt"), ["nova/"]);
}

#[test]
fn o_s3_diz_o_que_nao_sabe_fazer() {
    let servidor = servidor();
    servidor.colocar("bkt", "a.txt", b"a");
    let mut s3 = conectar(&servidor);

    assert!(!s3.caps().permissions && !s3.caps().symlinks && !s3.caps().set_modified);
    for erro in [
        s3.set_permissions("/a.txt", 0o644).unwrap_err(),
        s3.set_modified("/a.txt", UNIX_EPOCH).unwrap_err(),
        s3.symlink("/a.txt", "/l").unwrap_err(),
        s3.read_link("/a.txt").unwrap_err(),
    ] {
        assert_eq!(erro.kind(), io::ErrorKind::Unsupported, "{erro}");
    }
}

// ----- erros -----

#[test]
fn o_segredo_errado_diz_que_a_assinatura_nao_confere() {
    let servidor = servidor();
    let Err(erro) = S3::connect(S3Config {
        secret_key: "outro-segredo".into(),
        ..servidor.config(Some("bkt"))
    }) else {
        panic!("deveria falhar");
    };
    assert_eq!(erro.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        erro.to_string().contains("A assinatura não confere"),
        "{erro}"
    );
    assert!(erro.to_string().contains("SignatureDoesNotMatch"), "{erro}");
}

#[test]
fn a_chave_de_acesso_errada_e_dita() {
    let servidor = servidor();
    let Err(erro) = S3::connect(S3Config {
        access_key: "AKIAOUTRA".into(),
        ..servidor.config(Some("bkt"))
    }) else {
        panic!("deveria falhar");
    };
    assert!(erro.to_string().contains("chave de acesso"), "{erro}");
}

#[test]
fn um_bucket_inexistente_diz_que_o_bucket_nao_existe() {
    let servidor = servidor();
    let Err(erro) = S3::connect(servidor.config(Some("fantasma"))) else {
        panic!("deveria falhar");
    };
    assert_eq!(erro.kind(), io::ErrorKind::NotFound);
    assert!(erro.to_string().contains("O bucket não existe"), "{erro}");
}

#[test]
fn um_endereco_invalido_e_explicado() {
    let Err(erro) = S3::connect(S3Config::new("isto não é uma url", ACESSO, SEGREDO)) else {
        panic!("deveria falhar");
    };
    assert_eq!(erro.kind(), io::ErrorKind::InvalidInput);
    assert!(erro.to_string().contains("endereço"), "{erro}");
}

#[test]
fn as_chaves_nunca_aparecem_no_debug() {
    let servidor = servidor();
    let texto = format!("{:?}", servidor.config(Some("bkt")));
    assert!(
        !texto.contains(SEGREDO) && !texto.contains(ACESSO),
        "{texto}"
    );
}

#[test]
fn repete_o_pedido_quando_o_servidor_manda_slowdown() {
    let servidor = servidor();
    servidor.falhar_proximos(2);
    // Duas falhas passageiras: a terceira tentativa passa.
    let mut s3 = S3::connect(servidor.config(Some("bkt"))).unwrap();
    servidor.colocar("bkt", "a.txt", b"a");
    assert_eq!(nomes(&mut s3, "/"), ["a.txt"]);

    servidor.falhar_proximos(5);
    let erro = s3.list("/").unwrap_err();
    assert!(erro.to_string().contains("sobrecarregado"), "{erro}");
}

#[test]
fn troca_de_regiao_sozinho_quando_o_servidor_diz_qual_e() {
    let servidor = S3d::com_opcoes(Opcoes {
        regiao: "eu-west-1",
        ..Opcoes::default()
    });
    servidor.criar_bucket("bkt");
    servidor.colocar("bkt", "a.txt", b"a");
    let mut s3 = S3::connect(S3Config {
        region: "us-east-1".into(),
        ..servidor.config(Some("bkt"))
    })
    .unwrap();

    assert_eq!(nomes(&mut s3, "/"), ["a.txt"]);
    assert_eq!(ler_tudo(&mut s3, "/a.txt", 0), b"a");
}

// ----- link pré-assinado, extras -----

#[test]
fn o_link_pre_assinado_baixa_sem_credenciais_e_expira() {
    let servidor = servidor();
    servidor.colocar("bkt", "pasta/dado.bin", b"conteudo secreto");
    let s3 = conectar(&servidor);

    let link = s3
        .presign_get("/pasta/dado.bin", Duration::from_secs(1))
        .unwrap();
    assert!(link.contains("X-Amz-Signature="), "{link}");
    assert!(!link.contains(SEGREDO), "{link}");
    // Um GET puro, sem credenciais nenhumas.
    let corpo = ureq::get(&link)
        .call()
        .unwrap()
        .into_body()
        .read_to_string()
        .unwrap();
    assert_eq!(corpo, "conteudo secreto");

    std::thread::sleep(Duration::from_millis(2200));
    let erro = ureq::get(&link).call().unwrap_err();
    assert!(matches!(erro, ureq::Error::StatusCode(403)), "{erro}");

    // A validade tem limites.
    assert!(s3.presign_get("/pasta/dado.bin", Duration::ZERO).is_err());
    assert!(
        s3.presign_get("/pasta/dado.bin", Duration::from_secs(8 * 24 * 3600))
            .is_err()
    );
    assert!(s3.presign_get("/", Duration::from_secs(60)).is_err());
}

#[test]
fn head_conta_o_que_o_servidor_sabe_do_objeto() {
    let servidor = servidor();
    servidor.colocar("bkt", "notas.txt", b"12345");
    let mut s3 = conectar(&servidor);

    let info = s3.head("/notas.txt").unwrap();
    assert_eq!(info.size, 5);
    assert_eq!(info.storage_class, "STANDARD");
    assert!(info.etag.is_some_and(|etag| !etag.contains('"')));
    assert!(info.last_modified.is_some());
}

#[test]
fn set_storage_class_muda_o_head() {
    let servidor = servidor();
    servidor.colocar("bkt", "frio.bin", b"dados");
    let mut s3 = conectar(&servidor);

    s3.set_storage_class("/frio.bin", "STANDARD_IA").unwrap();

    assert_eq!(s3.head("/frio.bin").unwrap().storage_class, "STANDARD_IA");
    assert_eq!(servidor.classe("bkt", "frio.bin").unwrap(), "STANDARD_IA");
    assert_eq!(servidor.objeto("bkt", "frio.bin").unwrap(), b"dados");
}

#[test]
fn a_classe_da_conexao_vale_nos_envios() {
    let servidor = servidor();
    let mut s3 = S3::connect(S3Config {
        storage_class: Some("ONEZONE_IA".into()),
        ..servidor.config(Some("bkt"))
    })
    .unwrap();

    gravar(&mut s3, "/barato.bin", b"x");

    assert_eq!(servidor.classe("bkt", "barato.bin").unwrap(), "ONEZONE_IA");
}

#[test]
fn lista_e_aborta_os_envios_incompletos() {
    let servidor = servidor();
    {
        let mut s3 = S3::connect(com_partes_de_5_mib(&servidor)).unwrap();
        for nome in ["/a/um.bin", "/b/dois.bin"] {
            let mut envio = s3.open_write(nome, 0).unwrap();
            envio.write_all(&dados(5 * MIB)).unwrap();
        }
    }
    assert_eq!(servidor.envios_abertos(), 2);
    let mut s3 = conectar(&servidor);

    let todos = s3.pending_uploads("/").unwrap();
    assert_eq!(todos.len(), 2);
    let so_a = s3.pending_uploads("/a").unwrap();
    assert_eq!(so_a.len(), 1);
    assert_eq!(so_a[0].path, "/a/um.bin");
    assert!(so_a[0].initiated.is_some());

    s3.abort_upload(&so_a[0].path, &so_a[0].upload_id).unwrap();
    assert_eq!(servidor.envios_abertos(), 1);
    let resto = s3.pending_uploads("/").unwrap();
    assert_eq!(resto.len(), 1);
    assert_eq!(resto[0].path, "/b/dois.bin");
}

// ----- TLS -----

#[test]
fn o_certificado_autoassinado_so_passa_com_no_verify_ou_com_a_ca() {
    let servidor = S3d::com_opcoes(Opcoes {
        tls: true,
        ..Opcoes::default()
    });
    servidor.criar_bucket("bkt");
    servidor.colocar("bkt", "a.txt", b"a");

    let Err(erro) = S3::connect(S3Config {
        tls: TlsMode::Verify,
        ..servidor.config(Some("bkt"))
    }) else {
        panic!("o certificado autoassinado deveria ser recusado");
    };
    assert!(erro.to_string().contains("certificado"), "{erro}");

    let mut s3 = S3::connect(servidor.config(Some("bkt"))).unwrap();
    assert_eq!(nomes(&mut s3, "/"), ["a.txt"]);
    assert_eq!(ler_tudo(&mut s3, "/a.txt", 0), b"a");

    let pasta = Pasta::nova("s3-ca");
    let pem = pasta.caminho().join("ca.pem");
    std::fs::write(&pem, certificado_pem()).unwrap();
    let mut s3 = S3::connect(S3Config {
        tls: TlsMode::CustomCa(pem),
        ..servidor.config(Some("bkt"))
    })
    .unwrap();
    assert_eq!(nomes(&mut s3, "/"), ["a.txt"]);
}

// ----- a fila do M30 sobre o S3 -----

#[test]
fn a_fila_envia_e_baixa_pelo_s3() {
    let servidor = servidor();
    let local = Pasta::nova("s3-fila");
    let conteudo = dados(300_000);
    local.escrever("sobe.bin", &conteudo);
    let opener = servidor.config(Some("bkt")).opener();
    let fila = Transfers::new(2);

    fila.add(
        (Endpoint::Local, local.texto("sobe.bin")),
        (
            Endpoint::Remote(opener.clone()),
            "/pasta/sobe.bin".to_owned(),
        ),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(Duration::from_secs(30)));
    assert_eq!(servidor.objeto("bkt", "pasta/sobe.bin").unwrap(), conteudo);

    fila.add(
        (Endpoint::Remote(opener), "/pasta/sobe.bin".to_owned()),
        (Endpoint::Local, local.texto("desce.bin")),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(Duration::from_secs(30)));
    assert_eq!(
        std::fs::read(local.caminho().join("desce.bin")).unwrap(),
        conteudo
    );
    assert!(
        fila.items()
            .get_untracked()
            .iter()
            .all(|item| item.state == TransferState::Done)
    );
}

#[test]
fn a_fila_retoma_o_envio_interrompido_do_s3() {
    let servidor = servidor();
    let local = Pasta::nova("s3-fila-retoma");
    let conteudo = dados(15 * MIB);
    local.escrever("grande.bin", &conteudo);
    {
        // Deixa as duas primeiras partes no servidor, como se o app tivesse caído.
        let mut s3 = S3::connect(com_partes_de_5_mib(&servidor)).unwrap();
        let mut envio = s3.open_write("/grande.bin", 0).unwrap();
        envio.write_all(&conteudo[..10 * MIB]).unwrap();
    }
    assert_eq!(servidor.envios_abertos(), 1);
    servidor.limpar_pedidos();
    let opener = com_partes_de_5_mib(&servidor).opener();
    let fila = Transfers::new(1);

    fila.add(
        (Endpoint::Local, local.texto("grande.bin")),
        (Endpoint::Remote(opener), "/grande.bin".to_owned()),
        Conflict::Resume,
    );
    assert!(fila.wait_idle(Duration::from_secs(30)));

    assert_eq!(servidor.objeto("bkt", "grande.bin").unwrap(), conteudo);
    assert_eq!(servidor.envios_abertos(), 0);
    // Só a terceira parte foi enviada agora.
    assert_eq!(servidor.contar("PUT", "partNumber"), 1);
}
