//! O `Ftp` contra o servidor de teste (`comum/ftpd.rs`), que imita o vsftpd, o proftpd e o
//! pure-ftpd, sem rede de verdade.
#![cfg(all(unix, feature = "ftp"))]

mod comum;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui_files::ftp::{
    CertificateInfo, Ftp, FtpBuilder, FtpEncoding, FtpMode, FtpSecurity, TlsVersion,
};
use tomni_ui_files::{Conflict, Endpoint, EntryKind, Fs, Remote, TransferState, Transfers};

use comum::ftpd::{Ftpd, Opcoes, Tls, Versoes};
use comum::{Pasta, dados, esperar};

fn builder(servidor: &Ftpd) -> FtpBuilder {
    FtpBuilder::new("127.0.0.1", servidor.porta)
        .login("ana", "senha")
        .security(FtpSecurity::None)
        .timeout(Duration::from_secs(15))
}

/// O builder com TLS que aceita o certificado autoassinado do servidor de teste.
fn builder_tls(servidor: &Ftpd, seguranca: FtpSecurity) -> FtpBuilder {
    builder(servidor)
        .security(seguranca)
        .check_certificate(|_| true)
}

fn conectar(servidor: &Ftpd) -> Ftp {
    builder(servidor).connect().expect("conectar")
}

fn ler_tudo(fs: &mut dyn Fs, caminho: &str, offset: u64) -> Vec<u8> {
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

// ----- login -----

#[test]
fn login_com_senha_anonimo_e_conta() {
    let pasta = Pasta::nova("ftp-login");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    let mut ftp = conectar(&servidor);
    assert_eq!(ftp.home().unwrap(), "/");

    let anonimo = FtpBuilder::new("127.0.0.1", servidor.porta)
        .login("anonymous", "a@b.c")
        .security(FtpSecurity::None)
        .connect();
    assert!(anonimo.is_ok());

    let conta = FtpBuilder::new("127.0.0.1", servidor.porta)
        .login("conta", "senha")
        .account("projeto")
        .security(FtpSecurity::None)
        .connect();
    assert!(conta.is_ok());
    assert!(servidor.estado.contar("ACCT") >= 1);
}

#[test]
fn senha_errada_vira_erro_sem_a_senha() {
    let pasta = Pasta::nova("ftp-senha");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    let erro = match builder(&servidor).login("ana", "segredo123").connect() {
        Ok(_) => panic!("deveria falhar"),
        Err(erro) => erro,
    };
    assert_eq!(erro.kind(), io::ErrorKind::PermissionDenied);
    assert!(!erro.to_string().contains("segredo123"), "{erro}");
    // A senha nunca vai para o registro do servidor de teste.
    assert!(
        servidor
            .estado
            .comandos()
            .iter()
            .all(|c| !c.contains("segredo123"))
    );
}

// ----- listagens -----

fn preparar_arvore(servidor: &Ftpd) {
    servidor.escrever("a.txt", b"abc");
    servidor.escrever("pasta/b.txt", b"12345");
    std::os::unix::fs::symlink("a.txt", servidor.raiz.join("link")).unwrap();
    std::fs::set_permissions(
        servidor.raiz.join("a.txt"),
        std::os::unix::fs::PermissionsExt::from_mode(0o640),
    )
    .unwrap();
}

fn conferir_listagem(ftp: &mut Ftp) {
    let lista = ftp.list("/").unwrap();
    let a = lista.iter().find(|e| e.name == "a.txt").unwrap();
    assert_eq!(a.kind, EntryKind::File);
    assert_eq!(a.size, 3);
    assert!(a.modified.is_some());
    let pasta = lista.iter().find(|e| e.name == "pasta").unwrap();
    assert_eq!(pasta.kind, EntryKind::Dir);
    let link = lista.iter().find(|e| e.name == "link").unwrap();
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.link_target.as_deref(), Some("a.txt"));
    assert!(lista.iter().all(|e| e.name != "." && e.name != ".."));
}

#[test]
fn lista_com_mlsd_e_permissoes() {
    let pasta = Pasta::nova("ftp-mlsd");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd());
    preparar_arvore(&servidor);
    let mut ftp = conectar(&servidor);
    conferir_listagem(&mut ftp);
    assert!(servidor.estado.contar("MLSD") >= 1);
    let a = ftp
        .list("/")
        .unwrap()
        .into_iter()
        .find(|e| e.name == "a.txt")
        .unwrap();
    assert_eq!(a.permissions, Some(0o640));
    assert!(a.owner.is_some());
}

#[test]
fn lista_com_list_quando_nao_ha_mlsd() {
    let pasta = Pasta::nova("ftp-list");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    preparar_arvore(&servidor);
    let mut ftp = conectar(&servidor);
    conferir_listagem(&mut ftp);
    assert_eq!(servidor.estado.contar("MLSD"), 0);
    assert!(servidor.estado.contar("LIST") >= 1);
    let a = ftp
        .list("/")
        .unwrap()
        .into_iter()
        .find(|e| e.name == "a.txt")
        .unwrap();
    assert_eq!(a.permissions, Some(0o640));
}

#[test]
fn lista_do_pureftpd() {
    let pasta = Pasta::nova("ftp-pure");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::pureftpd());
    preparar_arvore(&servidor);
    let mut ftp = conectar(&servidor);
    conferir_listagem(&mut ftp);
}

#[test]
fn stat_de_arquivo_pasta_e_ausente() {
    for opcoes in [Opcoes::vsftpd(), Opcoes::proftpd()] {
        let pasta = Pasta::nova("ftp-stat");
        let servidor = Ftpd::novo(pasta.caminho(), opcoes);
        preparar_arvore(&servidor);
        let mut ftp = conectar(&servidor);
        let a = ftp.stat("/a.txt").unwrap();
        assert_eq!((a.kind, a.size), (EntryKind::File, 3));
        assert_eq!(ftp.stat("/pasta").unwrap().kind, EntryKind::Dir);
        let erro = ftp.stat("/nao-existe").unwrap_err();
        assert_eq!(erro.kind(), io::ErrorKind::NotFound);
    }
}

#[test]
fn datas_do_list_respeitam_o_fuso() {
    let pasta = Pasta::nova("ftp-fuso");
    let mut opcoes = Opcoes::vsftpd();
    opcoes.fuso_min = 180;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    let caminho = servidor.escrever("a.txt", b"x");
    // Recente (o LIST só mostra a hora nas datas dos últimos seis meses).
    let alvo = std::time::SystemTime::now() - Duration::from_secs(2 * 86_400);
    let t = UNIX_EPOCH + Duration::from_secs(alvo.duration_since(UNIX_EPOCH).unwrap().as_secs());
    std::fs::File::open(&caminho)
        .unwrap()
        .set_modified(t)
        .unwrap();
    // O servidor escreve a hora local (UTC+3); o cliente sabe do fuso e acerta.
    let mut ftp = builder(&servidor).timezone_offset(180).connect().unwrap();
    let a = ftp
        .list("/")
        .unwrap()
        .into_iter()
        .find(|e| e.name == "a.txt")
        .unwrap();
    let achada = a.modified.unwrap().duration_since(UNIX_EPOCH).unwrap();
    // O LIST não tem segundos.
    assert!(
        achada
            .as_secs()
            .abs_diff(t.duration_since(UNIX_EPOCH).unwrap().as_secs())
            < 60,
        "{achada:?}"
    );
}

// ----- transferências -----

#[test]
fn baixar_e_enviar_5_mib_em_cada_perfil() {
    for opcoes in [Opcoes::vsftpd(), Opcoes::proftpd(), Opcoes::pureftpd()] {
        let pasta = Pasta::nova("ftp-5mib");
        let servidor = Ftpd::novo(pasta.caminho(), opcoes);
        let conteudo = dados(5 * 1024 * 1024);
        servidor.escrever("grande.bin", &conteudo);
        let mut ftp = conectar(&servidor);
        assert_eq!(ler_tudo(&mut ftp, "/grande.bin", 0), conteudo);
        gravar(&mut ftp, "/enviado.bin", 0, &conteudo);
        assert_eq!(servidor.ler("enviado.bin"), conteudo);
    }
}

#[test]
fn modo_ativo_baixa_e_envia() {
    let pasta = Pasta::nova("ftp-ativo");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd());
    let conteudo = dados(300_000);
    servidor.escrever("a.bin", &conteudo);
    let mut ftp = builder(&servidor).mode(FtpMode::Active).connect().unwrap();
    assert_eq!(ler_tudo(&mut ftp, "/a.bin", 0), conteudo);
    gravar(&mut ftp, "/b.bin", 0, &conteudo);
    assert_eq!(servidor.ler("b.bin"), conteudo);
    assert!(servidor.estado.contar("EPRT") >= 1);
}

#[test]
fn modo_ativo_cai_no_port_sem_eprt() {
    let pasta = Pasta::nova("ftp-port");
    let mut opcoes = Opcoes::vsftpd();
    opcoes.eprt_recusado = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let mut ftp = builder(&servidor).mode(FtpMode::Active).connect().unwrap();
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    assert!(servidor.estado.contar("PORT") >= 1);
}

#[test]
fn epsv_recusado_cai_no_pasv() {
    let pasta = Pasta::nova("ftp-epsv");
    let mut opcoes = Opcoes::vsftpd();
    opcoes.epsv_recusado = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let mut ftp = conectar(&servidor);
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    assert!(servidor.estado.contar("PASV") >= 1);
}

#[test]
fn ip_privado_no_pasv_vira_o_do_par() {
    let pasta = Pasta::nova("ftp-nat");
    let mut opcoes = Opcoes::vsftpd();
    opcoes.epsv_recusado = true;
    opcoes.ip_pasv = Some("10.0.0.1".parse().unwrap());
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let mut ftp = conectar(&servidor);
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
}

#[test]
fn retoma_download_e_upload() {
    for opcoes in [Opcoes::proftpd(), Opcoes::vsftpd()] {
        let pasta = Pasta::nova("ftp-retomar");
        let servidor = Ftpd::novo(pasta.caminho(), opcoes);
        let conteudo = dados(100_000);
        servidor.escrever("a.bin", &conteudo);
        let mut ftp = conectar(&servidor);
        assert_eq!(ler_tudo(&mut ftp, "/a.bin", 40_000), conteudo[40_000..]);

        servidor.escrever("parcial.bin", &conteudo[..60_000]);
        gravar(&mut ftp, "/parcial.bin", 60_000, &conteudo[60_000..]);
        assert_eq!(servidor.ler("parcial.bin"), conteudo);
        assert!(ftp.caps().resume_upload);
    }
}

#[test]
fn retoma_o_upload_por_appe_sem_rest_stream() {
    let pasta = Pasta::nova("ftp-appe");
    let mut opcoes = Opcoes::vsftpd();
    opcoes.sem_rest_stream = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    let conteudo = dados(50_000);
    servidor.escrever("parcial.bin", &conteudo[..20_000]);
    let mut ftp = conectar(&servidor);
    gravar(&mut ftp, "/parcial.bin", 20_000, &conteudo[20_000..]);
    assert_eq!(servidor.ler("parcial.bin"), conteudo);
    assert!(servidor.estado.contar("APPE") >= 1);
}

#[test]
fn cancelar_no_meio_e_o_proximo_comando_funciona() {
    let pasta = Pasta::nova("ftp-abor");
    let mut opcoes = Opcoes::proftpd();
    opcoes.lento = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("grande.bin", &dados(4 * 1024 * 1024));
    servidor.escrever("a.txt", b"oi");
    let mut ftp = conectar(&servidor);
    {
        let mut arquivo = ftp.open_read("/grande.bin", 0).unwrap();
        let mut bloco = vec![0u8; 4096];
        arquivo.read_exact(&mut bloco).unwrap();
        // Soltar sem `finish` cancela.
    }
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    assert_eq!(servidor.estado.controles.load(Ordering::SeqCst), 1);
    assert!(servidor.estado.contar("ABOR") >= 1);
}

// ----- o resto do Fs -----

#[test]
fn operacoes_de_arquivo_e_pasta() {
    let pasta = Pasta::nova("ftp-fs");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd());
    servidor.escrever("a.txt", b"a");
    servidor.escrever("b.txt", b"b");
    let mut ftp = conectar(&servidor);
    ftp.mkdir("/nova").unwrap();
    assert!(servidor.raiz.join("nova").is_dir());
    ftp.rename("/a.txt", "/nova/a.txt", false).unwrap();
    assert!(servidor.raiz.join("nova/a.txt").exists());
    ftp.remove_file("/nova/a.txt").unwrap();
    ftp.remove_dir("/nova").unwrap();
    assert!(!servidor.raiz.join("nova").exists());
    assert_eq!(
        ftp.remove_file("/nao-existe").unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn rename_sem_sobrescrever_recusa_quando_o_destino_existe() {
    let pasta = Pasta::nova("ftp-rename");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    servidor.escrever("a.txt", b"a");
    servidor.escrever("b.txt", b"b");
    let mut ftp = conectar(&servidor);
    let erro = ftp.rename("/a.txt", "/b.txt", false).unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(servidor.ler("b.txt"), b"b");
    ftp.rename("/a.txt", "/b.txt", true).unwrap();
    assert_eq!(servidor.ler("b.txt"), b"a");
}

#[test]
fn set_modified_pelos_tres_caminhos() {
    let alvo = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    for (nome, opcoes) in [("mfmt", Opcoes::pureftpd()), ("mdtm", Opcoes::vsftpd())] {
        let pasta = Pasta::nova("ftp-data");
        let servidor = Ftpd::novo(pasta.caminho(), opcoes);
        let caminho = servidor.escrever("a.txt", b"x");
        let mut ftp = conectar(&servidor);
        ftp.set_modified("/a.txt", alvo).unwrap();
        let obtido = std::fs::metadata(caminho).unwrap().modified().unwrap();
        assert_eq!(obtido, alvo, "{nome}");
    }
}

#[test]
fn site_chmod_e_symlink() {
    let pasta = Pasta::nova("ftp-site");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd());
    servidor.escrever("a.txt", b"x");
    let mut ftp = conectar(&servidor);
    assert!(ftp.caps().permissions);
    assert!(ftp.caps().symlinks);
    ftp.set_permissions("/a.txt", 0o600).unwrap();
    let modo = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(servidor.raiz.join("a.txt"))
            .unwrap()
            .permissions(),
    );
    assert_eq!(modo & 0o7777, 0o600);
    ftp.symlink("a.txt", "/atalho").unwrap();
    assert_eq!(
        std::fs::read_link(servidor.raiz.join("atalho")).unwrap(),
        std::path::Path::new("a.txt")
    );
}

#[test]
fn sem_symlink_quando_o_servidor_nao_sabe() {
    let pasta = Pasta::nova("ftp-sem-symlink");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    let mut ftp = conectar(&servidor);
    assert!(!ftp.caps().symlinks);
    assert_eq!(
        ftp.symlink("a", "/b").unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
}

// ----- nomes e codificação -----

#[test]
fn nomes_com_acento_espaco_e_cerquilha_em_utf8() {
    let pasta = Pasta::nova("ftp-utf8");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd());
    for nome in ["ação.txt", "com espaço.txt", "#nota.txt"] {
        servidor.escrever(nome, nome.as_bytes());
    }
    let mut ftp = conectar(&servidor);
    assert_eq!(
        nomes(&mut ftp, "/"),
        ["#nota.txt", "ação.txt", "com espaço.txt"]
    );
    assert_eq!(ler_tudo(&mut ftp, "/ação.txt", 0), "ação.txt".as_bytes());
    gravar(&mut ftp, "/novo ç#.txt", 0, b"ok");
    assert_eq!(servidor.ler("novo ç#.txt"), b"ok");
}

#[test]
fn nomes_em_latin1() {
    let pasta = Pasta::nova("ftp-latin1");
    let mut opcoes = Opcoes::vsftpd();
    opcoes.latin1 = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    // Para o servidor, o nome é o texto Latin-1; no disco guardamos o mesmo texto.
    servidor.escrever("ação.txt", b"oi");
    for encoding in [FtpEncoding::Auto, FtpEncoding::Latin1] {
        let mut ftp = builder(&servidor).encoding(encoding).connect().unwrap();
        assert_eq!(nomes(&mut ftp, "/"), ["ação.txt"]);
        assert_eq!(ler_tudo(&mut ftp, "/ação.txt", 0), b"oi");
    }
}

// ----- reconexão -----

#[test]
fn o_421_reconecta_e_repete() {
    let pasta = Pasta::nova("ftp-421");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd());
    servidor.escrever("a.txt", b"oi");
    let mut ftp = conectar(&servidor);
    nomes(&mut ftp, "/");
    servidor.derrubar_proximo();
    assert_eq!(nomes(&mut ftp, "/"), ["a.txt"]);
    assert_eq!(servidor.estado.controles.load(Ordering::SeqCst), 2);
}

#[test]
fn conexao_derrubada_reconecta() {
    let pasta = Pasta::nova("ftp-queda");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    servidor.escrever("a.txt", b"oi");
    let mut ftp = conectar(&servidor);
    nomes(&mut ftp, "/");
    servidor.derrubar_todas();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(nomes(&mut ftp, "/"), ["a.txt"]);
}

#[test]
fn keepalive_manda_noop() {
    let pasta = Pasta::nova("ftp-keepalive");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    let mut ftp = conectar(&servidor);
    ftp.keepalive().unwrap();
    assert_eq!(servidor.estado.contar("NOOP"), 1);
}

// ----- TLS -----

fn conferir_tls(servidor: &Ftpd, builder: FtpBuilder, versao: &str) {
    let conteudo = dados(5 * 1024 * 1024);
    servidor.escrever("grande.bin", &conteudo);
    let mut ftp = builder.connect().unwrap();
    assert!(ftp.is_secure());
    assert_eq!(ler_tudo(&mut ftp, "/grande.bin", 0), conteudo);
    gravar(&mut ftp, "/enviado.bin", 0, &conteudo);
    assert_eq!(servidor.ler("enviado.bin"), conteudo);
    assert!(
        servidor
            .estado
            .versoes
            .lock()
            .unwrap()
            .iter()
            .all(|v| v == versao)
    );
    assert!(servidor.estado.dados_retomados.load(Ordering::SeqCst) >= 2);
    assert_eq!(servidor.estado.recusados.load(Ordering::SeqCst), 0);
}

#[test]
fn tls_explicito_1_3_com_retomada() {
    for opcoes in [Opcoes::vsftpd(), Opcoes::proftpd(), Opcoes::pureftpd()] {
        let pasta = Pasta::nova("ftp-tls13");
        let servidor = Ftpd::novo(pasta.caminho(), opcoes.tls(Tls::Explicito));
        conferir_tls(
            &servidor,
            builder_tls(&servidor, FtpSecurity::Explicit),
            "1.3",
        );
    }
}

#[test]
fn tls_explicito_1_2_com_retomada() {
    for opcoes in [Opcoes::vsftpd(), Opcoes::proftpd()] {
        let pasta = Pasta::nova("ftp-tls12");
        let opcoes = opcoes.tls(Tls::Explicito).versoes(Versoes::So12);
        let servidor = Ftpd::novo(pasta.caminho(), opcoes);
        conferir_tls(
            &servidor,
            builder_tls(&servidor, FtpSecurity::Explicit),
            "1.2",
        );
    }
}

#[test]
fn tls_implicito() {
    let pasta = Pasta::nova("ftp-implicito");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd().tls(Tls::Implicito));
    conferir_tls(
        &servidor,
        builder_tls(&servidor, FtpSecurity::Implicit),
        "1.3",
    );
}

#[test]
fn varias_conexoes_de_dados_seguidas_no_tls_1_3() {
    let pasta = Pasta::nova("ftp-tiquetes");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd().tls(Tls::Explicito));
    servidor.escrever("a.txt", b"oi");
    let mut ftp = builder_tls(&servidor, FtpSecurity::Explicit)
        .connect()
        .unwrap();
    for _ in 0..12 {
        assert_eq!(nomes(&mut ftp, "/"), ["a.txt"]);
        assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    }
    assert_eq!(servidor.estado.dados_completos.load(Ordering::SeqCst), 0);
    assert_eq!(servidor.estado.recusados.load(Ordering::SeqCst), 0);
}

#[test]
fn fallback_para_1_2_quando_o_servidor_nao_retoma_no_1_3() {
    let pasta = Pasta::nova("ftp-fallback");
    let mut opcoes = Opcoes::vsftpd().tls(Tls::Explicito);
    opcoes.sem_tiquetes_13 = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let mut ftp = builder_tls(&servidor, FtpSecurity::Explicit)
        .connect()
        .unwrap();
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    let aviso = ftp.take_notice().expect("o fallback avisa");
    assert!(aviso.contains("TLS 1.2"), "{aviso}");
    assert!(ftp.take_notice().is_none());
    assert!(servidor.estado.dados_retomados.load(Ordering::SeqCst) >= 1);
}

#[test]
fn tls_forcado_1_3_sem_fallback_falha_com_mensagem() {
    let pasta = Pasta::nova("ftp-sem-fallback");
    let mut opcoes = Opcoes::vsftpd().tls(Tls::Explicito);
    opcoes.sem_tiquetes_13 = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let mut ftp = builder_tls(&servidor, FtpSecurity::Explicit)
        .tls_version(TlsVersion::Only13)
        .connect()
        .unwrap();
    assert!(ftp.open_read("/a.txt", 0).is_err());
}

#[test]
fn tls_se_disponivel_cai_para_claro() {
    let pasta = Pasta::nova("ftp-se-disponivel");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    servidor.escrever("a.txt", b"oi");
    let mut ftp = builder(&servidor)
        .security(FtpSecurity::ExplicitIfAvailable)
        .connect()
        .unwrap();
    assert!(!ftp.is_secure());
    assert_eq!(
        ftp.take_notice().as_deref(),
        Some(tomni_ui_files::ftp::NOTICE_NO_TLS)
    );
    assert_eq!(ftp.take_notice(), None);
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
}

#[test]
fn tls_exigido_sem_suporte_do_servidor_e_erro() {
    let pasta = Pasta::nova("ftp-sem-tls");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    let erro = builder(&servidor)
        .security(FtpSecurity::Explicit)
        .check_certificate(|_| true)
        .connect()
        .err()
        .expect("deveria falhar");
    assert!(erro.to_string().contains("TLS"), "{erro}");
}

#[test]
fn certificado_autoassinado_pede_confirmacao() {
    let pasta = Pasta::nova("ftp-cert");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd().tls(Tls::Explicito));
    servidor.escrever("a.txt", b"oi");

    // Sem `check_certificate`, o certificado desconhecido é um erro.
    let sem = builder(&servidor)
        .security(FtpSecurity::Explicit)
        .connect()
        .err()
        .expect("deveria falhar");
    assert!(!sem.to_string().is_empty());

    // Com a pergunta recusada, também.
    let recusado = builder(&servidor)
        .security(FtpSecurity::Explicit)
        .check_certificate(|_| false)
        .connect();
    assert!(recusado.is_err());

    // Aceito: a pergunta recebe a impressão certa e a sessão funciona.
    let recebido: Arc<std::sync::Mutex<Vec<CertificateInfo>>> = Arc::default();
    let guardar = recebido.clone();
    let mut ftp = builder(&servidor)
        .security(FtpSecurity::Explicit)
        .check_certificate(move |info| {
            guardar.lock().unwrap().push(info.clone());
            true
        })
        .connect()
        .unwrap();
    assert_eq!(ler_tudo(&mut ftp, "/a.txt", 0), b"oi");
    let infos = recebido.lock().unwrap();
    // A pergunta é feita uma vez por sessão: as conexões de dados repetem a impressão.
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].sha256, comum::ftpd::impressao_do_certificado());
    assert_eq!(infos[0].host, "127.0.0.1");
}

#[test]
fn dados_com_outro_certificado_falham() {
    let pasta = Pasta::nova("ftp-cert-dados");
    let mut opcoes = Opcoes::pureftpd().tls(Tls::Explicito);
    opcoes.cert_dados_diferente = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let perguntas = Arc::new(AtomicUsize::new(0));
    let contar = perguntas.clone();
    let mut ftp = builder(&servidor)
        .security(FtpSecurity::Explicit)
        .check_certificate(move |_| {
            contar.fetch_add(1, Ordering::SeqCst);
            true
        })
        .connect()
        .unwrap();
    assert!(ftp.open_read("/a.txt", 0).is_err());
    assert_eq!(perguntas.load(Ordering::SeqCst), 1);
}

// ----- proxy -----

#[test]
fn ativo_pelo_proxy_da_erro_claro() {
    let pasta = Pasta::nova("ftp-proxy-ativo");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::vsftpd());
    servidor.escrever("a.txt", b"oi");
    let proxy = tomni_ui_net::Proxy::Socks5 {
        host: "127.0.0.1".to_owned(),
        port: 9,
        user: None,
        password: None,
    };
    let resultado = builder(&servidor)
        .mode(FtpMode::Active)
        .proxy(proxy)
        .connect();
    assert!(resultado.is_err());
}

// ----- a fila do M30 sobre o FTP -----

#[test]
fn a_fila_baixa_varios_arquivos_com_uma_conexao_por_trabalhador() {
    let pasta = Pasta::nova("ftp-fila");
    let local = Pasta::nova("ftp-fila-local");
    let servidor = Ftpd::novo(pasta.caminho(), Opcoes::proftpd().tls(Tls::Explicito));
    let mut esperado = Vec::new();
    for i in 0..6 {
        let conteudo = dados(200_000 + i);
        servidor.escrever(&format!("pasta/arq{i}.bin"), &conteudo);
        esperado.push(conteudo);
    }
    let opener = builder_tls(&servidor, FtpSecurity::Explicit).opener();
    let fila = Transfers::new(2);
    let ids: Vec<u64> = (0..6)
        .map(|i| {
            fila.add(
                (
                    Endpoint::Remote(opener.clone()),
                    format!("/pasta/arq{i}.bin"),
                ),
                (Endpoint::Local, local.texto(&format!("arq{i}.bin"))),
                Conflict::Overwrite,
            )
        })
        .collect();
    assert!(fila.wait_idle(Duration::from_secs(30)));
    for (i, id) in ids.iter().enumerate() {
        let item = fila
            .items()
            .get_untracked()
            .into_iter()
            .find(|item| item.id == *id)
            .unwrap();
        assert_eq!(item.state, TransferState::Done, "{item:?}");
        assert_eq!(
            std::fs::read(local.caminho().join(format!("arq{i}.bin"))).unwrap(),
            esperado[i]
        );
    }
    // Uma conexão de controle por trabalhador, não uma por arquivo.
    assert_eq!(servidor.estado.controles.load(Ordering::SeqCst), 2);
}

#[test]
fn o_limite_de_conexoes_do_servidor_reduz_os_trabalhadores() {
    let pasta = Pasta::nova("ftp-limite");
    let local = Pasta::nova("ftp-limite-local");
    let mut opcoes = Opcoes::pureftpd();
    opcoes.max_conexoes = Some(2);
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    for i in 0..6 {
        servidor.escrever(&format!("arq{i}.bin"), &dados(150_000));
    }
    // A conexão da tela (a `Remote`) já ocupa uma das duas.
    let mut tela = conectar(&servidor);
    let opener = builder(&servidor).opener();
    let fila = Transfers::new(3);
    let ids: Vec<u64> = (0..6)
        .map(|i| {
            fila.add(
                (Endpoint::Remote(opener.clone()), format!("/arq{i}.bin")),
                (Endpoint::Local, local.texto(&format!("arq{i}.bin"))),
                Conflict::Overwrite,
            )
        })
        .collect();
    assert!(fila.wait_idle(Duration::from_secs(30)));
    let itens = fila.items().get_untracked();
    for id in &ids {
        let item = itens.iter().find(|item| item.id == *id).unwrap();
        assert_eq!(item.state, TransferState::Done, "{item:?}");
    }
    assert!(
        itens.iter().any(|item| item
            .notice
            .as_deref()
            .is_some_and(|aviso| aviso.contains("limita as conexões"))),
        "{itens:?}"
    );
    for i in 0..6 {
        assert!(local.caminho().join(format!("arq{i}.bin")).exists());
    }
    // A tela continua funcionando.
    assert_eq!(nomes(&mut tela, "/").len(), 6);
}

#[test]
fn a_remote_repassa_o_aviso_do_fallback() {
    let pasta = Pasta::nova("ftp-remote-aviso");
    let mut opcoes = Opcoes::vsftpd().tls(Tls::Explicito);
    opcoes.sem_tiquetes_13 = true;
    let servidor = Ftpd::novo(pasta.caminho(), opcoes);
    servidor.escrever("a.txt", b"oi");
    let remote = Remote::spawn(
        "ftp-teste",
        builder_tls(&servidor, FtpSecurity::Explicit).opener(),
    );
    assert!(remote.notice().get_untracked().is_none());
    let lista = esperar(&remote.list("/")).unwrap();
    assert_eq!(lista.len(), 1);
    let aviso = remote.notice().get_untracked().expect("o aviso chegou");
    assert!(aviso.contains("TLS 1.2"), "{aviso}");
}
