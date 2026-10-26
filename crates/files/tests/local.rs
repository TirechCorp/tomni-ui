//! O `Local` e o `MemFs`: o mesmo contrato do trait `Fs`.

mod comum;

use std::io::{Read, Write};
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui_files::{EntryKind, Fs, Local, MemFs, path};

use comum::{Pasta, dados};

fn ler(fs: &mut dyn Fs, caminho: &str, offset: u64) -> Vec<u8> {
    let mut arquivo = fs.open_read(caminho, offset).unwrap();
    let mut saida = Vec::new();
    arquivo.read_to_end(&mut saida).unwrap();
    arquivo.finish().unwrap();
    saida
}

fn gravar(fs: &mut dyn Fs, caminho: &str, offset: u64, conteudo: &[u8]) {
    let mut arquivo = fs.open_write(caminho, offset).unwrap();
    arquivo.write_all(conteudo).unwrap();
    arquivo.finish().unwrap();
}

/// O contrato que todo backend cumpre; roda sobre `base`, uma pasta que existe e está vazia.
fn contrato(fs: &mut dyn Fs, base: &str) {
    let arquivo = path::join(base, "relatório final.txt");
    gravar(fs, &arquivo, 0, b"ola mundo");
    assert_eq!(ler(fs, &arquivo, 0), b"ola mundo");
    assert_eq!(ler(fs, &arquivo, 4), b"mundo");

    // Retomar: grava a partir do deslocamento, sem esvaziar o começo.
    gravar(fs, &arquivo, 4, b"MUNDO!");
    assert_eq!(ler(fs, &arquivo, 0), b"ola MUNDO!");
    // Sem deslocamento, esvazia.
    gravar(fs, &arquivo, 0, b"x");
    assert_eq!(ler(fs, &arquivo, 0), b"x");

    let dados = fs.stat(&arquivo).unwrap();
    assert_eq!(dados.name, "relatório final.txt");
    assert_eq!(dados.kind, EntryKind::File);
    assert_eq!(dados.size, 1);

    // Pastas.
    let pasta = path::join(base, "sub pasta");
    fs.mkdir(&pasta).unwrap();
    assert_eq!(
        fs.mkdir(&pasta).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert!(fs.stat(&pasta).unwrap().is_dir());
    gravar(fs, &path::join(&pasta, "a.txt"), 0, b"a");

    let mut nomes: Vec<String> = fs.list(base).unwrap().into_iter().map(|e| e.name).collect();
    nomes.sort();
    assert_eq!(nomes, ["relatório final.txt", "sub pasta"]);

    // Apagar pasta com conteúdo é erro; vazia, não.
    assert!(fs.remove_dir(&pasta).is_err());
    fs.remove_file(&path::join(&pasta, "a.txt")).unwrap();
    fs.remove_dir(&pasta).unwrap();
    assert_eq!(
        fs.stat(&pasta).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );

    // Renomear: sem `overwrite` recusa, com `overwrite` troca.
    let outro = path::join(base, "outro.txt");
    gravar(fs, &outro, 0, b"outro");
    let erro = fs.rename(&arquivo, &outro, false).unwrap_err();
    assert_eq!(erro.kind(), std::io::ErrorKind::AlreadyExists);
    fs.rename(&arquivo, &outro, true).unwrap();
    assert_eq!(ler(fs, &outro, 0), b"x");
    assert!(fs.stat(&arquivo).is_err());

    // Erros em português, com o caminho.
    let faltando = path::join(base, "não existe");
    let erro = fs.stat(&faltando).unwrap_err();
    assert_eq!(erro.kind(), std::io::ErrorKind::NotFound);
    assert!(erro.to_string().contains("não existe"), "{erro}");
    fs.remove_file(&outro).unwrap();
    assert!(fs.list(base).unwrap().is_empty());
}

#[test]
fn o_local_cumpre_o_contrato() {
    let pasta = Pasta::nova("contrato");
    let base = pasta.texto("");
    contrato(&mut Local::new(), base.trim_end_matches('/'));
}

#[test]
fn o_memfs_cumpre_o_contrato() {
    let mem = MemFs::new();
    mem.dir("/dados");
    contrato(&mut mem.connect(), "/dados");
}

#[test]
fn a_data_e_as_permissoes_mudam() {
    let pasta = Pasta::nova("datas");
    pasta.escrever("a.txt", b"a");
    let caminho = pasta.texto("a.txt");
    let mut fs = Local::new();
    let data = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    fs.set_modified(&caminho, data).unwrap();
    assert_eq!(fs.stat(&caminho).unwrap().modified, Some(data));
    #[cfg(unix)]
    {
        fs.set_permissions(&caminho, 0o640).unwrap();
        assert_eq!(fs.stat(&caminho).unwrap().permissions, Some(0o640));
    }
}

#[cfg(unix)]
#[test]
fn real_path_resolve_links_e_acusa_o_que_nao_existe() {
    let pasta = Pasta::nova("real-path");
    pasta.escrever("dentro/alvo.txt", b"1");
    let mut fs = Local::new();
    let base = fs.real_path(&pasta.texto("")).unwrap();
    fs.symlink("dentro", &pasta.texto("atalho")).unwrap();
    assert_eq!(
        fs.real_path(&pasta.texto("atalho/../atalho/alvo.txt"))
            .unwrap(),
        path::join(&path::join(&base, "dentro"), "alvo.txt")
    );
    assert_eq!(
        fs.real_path(&pasta.texto("nada")).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[test]
fn links_simbolicos_sao_vistos_pelo_lstat_e_seguidos_pelo_stat() {
    let pasta = Pasta::nova("links");
    pasta.escrever("alvo.txt", b"12345");
    let link = pasta.texto("atalho");
    let mut fs = Local::new();
    fs.symlink("alvo.txt", &link).unwrap();
    assert_eq!(fs.lstat(&link).unwrap().kind, EntryKind::Symlink);
    assert_eq!(fs.read_link(&link).unwrap(), "alvo.txt");
    let seguido = fs.stat(&link).unwrap();
    assert_eq!(seguido.kind, EntryKind::File);
    assert_eq!(seguido.size, 5);
    let listado = fs
        .list(pasta.texto("").trim_end_matches('/'))
        .unwrap()
        .into_iter()
        .find(|e| e.name == "atalho")
        .unwrap();
    assert_eq!(listado.kind, EntryKind::Symlink);
    assert_eq!(listado.link_target.as_deref(), Some("alvo.txt"));
}

#[cfg(unix)]
#[test]
fn o_memfs_resolve_o_real_path_como_um_servidor() {
    use std::io::ErrorKind;
    let mut fs = MemFs::new();
    fs.put("/home/a/arquivo.txt", b"1")
        .put("/fora/segredo.txt", b"2")
        .dir("/home/vazia");
    fs.link("/home/para-fora", "/fora")
        .link("/home/cadeia", "para-fora")
        .link("/home/relativo", "a/../a")
        .link("/home/a/volta", "..");

    // Link para fora da pasta, e link que aponta para outro link.
    assert_eq!(
        fs.real_path("/home/para-fora/segredo.txt").unwrap(),
        "/fora/segredo.txt"
    );
    assert_eq!(
        fs.real_path("/home/cadeia/segredo.txt").unwrap(),
        "/fora/segredo.txt"
    );
    // `.` e `..` se juntam, inclusive o `..` que vem de dentro de um link.
    assert_eq!(
        fs.real_path("/home/./a/../a/arquivo.txt").unwrap(),
        "/home/a/arquivo.txt"
    );
    assert_eq!(
        fs.real_path("/home/relativo/arquivo.txt").unwrap(),
        "/home/a/arquivo.txt"
    );
    assert_eq!(fs.real_path("/home/a/volta/vazia").unwrap(), "/home/vazia");
    assert_eq!(fs.real_path("/home/../..").unwrap(), "/");
    // O que não existe, ou não é pasta onde precisa ser.
    assert_eq!(
        fs.real_path("/home/nada").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.real_path("/home/a/arquivo.txt/dentro")
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );

    // Um ciclo de links acaba em erro em vez de rodar para sempre.
    fs.link("/home/ida", "volta-ciclo")
        .link("/home/volta-ciclo", "ida");
    assert_eq!(
        fs.real_path("/home/ida").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );

    // O que está negado continua negado.
    fs.deny("/fora");
    assert_eq!(
        fs.real_path("/home/para-fora/segredo.txt")
            .unwrap_err()
            .kind(),
        ErrorKind::PermissionDenied
    );
}

#[test]
fn sem_permissao_vira_permission_denied_em_portugues() {
    use std::os::unix::fs::PermissionsExt;
    let pasta = Pasta::nova("negado");
    let privada = pasta.caminho().join("privada");
    std::fs::create_dir(&privada).unwrap();
    std::fs::write(privada.join("x"), b"x").unwrap();
    std::fs::set_permissions(&privada, std::fs::Permissions::from_mode(0o000)).unwrap();
    let resultado = Local::new().list(&pasta.texto("privada"));
    std::fs::set_permissions(&privada, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Quem roda como root enxerga tudo; o teste só vale para os demais.
    if let Err(erro) = resultado {
        assert_eq!(erro.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(erro.to_string().contains("sem permissão"), "{erro}");
    }
}

#[test]
fn o_memfs_nega_o_que_foi_negado_e_deixa_o_resto() {
    let mem = MemFs::new();
    mem.put("/segredo/a.txt", b"a").put("/livre/b.txt", b"b");
    mem.deny("/segredo");
    let mut fs = mem.connect();
    let erro = fs.list("/segredo").unwrap_err();
    assert_eq!(erro.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(erro.to_string().contains("sem permissão"));
    assert_eq!(fs.list("/livre").unwrap().len(), 1);
}

#[test]
fn dados_grandes_voltam_inteiros_em_blocos_pequenos() {
    let mem = MemFs::new();
    mem.set_chunk(7);
    let original = dados(1000);
    let mut fs = mem.connect();
    gravar(&mut fs, "/home/ana/x.bin", 0, &original);
    assert_eq!(ler(&mut fs, "/home/ana/x.bin", 0), original);
}
