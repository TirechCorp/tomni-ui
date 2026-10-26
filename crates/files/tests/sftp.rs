//! O `Sftp` contra um servidor `russh` local que serve uma pasta de verdade.
#![cfg(all(unix, feature = "sftp"))]

mod comum;

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use tomni_ui_files::sftp::Sftp;
use tomni_ui_files::{
    Conflict, Endpoint, EntryKind, Fs, Opener, Remote, RemoteState, TransferState, Transfers,
};
use tomni_ui_net::Proxy;

use comum::servidor::Servidor;
use comum::{Pasta, dados, esperar};

const ESPERA: Duration = Duration::from_secs(30);

fn conectar(servidor: &Servidor) -> Sftp {
    Sftp::connect(&servidor.builder()).expect("conectar ao servidor de teste")
}

fn opener(servidor: &Servidor) -> Opener {
    let builder = servidor.builder();
    Arc::new(move || Ok(Box::new(Sftp::connect(&builder)?) as Box<dyn Fs>))
}

fn ler_tudo(sftp: &mut Sftp, caminho: &str, offset: u64) -> Vec<u8> {
    let mut arquivo = sftp.open_read(caminho, offset).unwrap();
    let mut dados = Vec::new();
    arquivo.read_to_end(&mut dados).unwrap();
    arquivo.finish().unwrap();
    dados
}

fn gravar(sftp: &mut Sftp, caminho: &str, offset: u64, dados: &[u8]) {
    let mut arquivo = sftp.open_write(caminho, offset).unwrap();
    arquivo.write_all(dados).unwrap();
    arquivo.finish().unwrap();
}

#[test]
fn entra_com_senha_e_acha_a_casa() {
    let pasta = Pasta::nova("sftp-senha");
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);
    assert_eq!(sftp.home().unwrap(), "/home");
    assert!(sftp.list("/home").unwrap().is_empty());
}

#[test]
fn entra_com_chave() {
    let pasta = Pasta::nova("sftp-chave");
    let servidor = Servidor::novo(pasta.caminho());
    let builder = tomni_ui_terminal::ssh::SshBuilder::new("127.0.0.1", servidor.porta, "ana")
        .known_hosts(servidor.known_hosts())
        .key_file(servidor.arquivo_da_chave(), None);
    let mut sftp = Sftp::connect(&builder).unwrap();
    assert_eq!(sftp.home().unwrap(), "/home");
}

#[test]
fn senha_errada_diz_o_motivo() {
    let pasta = Pasta::nova("sftp-errada");
    let servidor = Servidor::novo(pasta.caminho());
    let builder = servidor.builder().password("outra");
    let erro = Sftp::connect(&builder).err().expect("deve recusar");
    assert!(erro.to_string().contains("autenticação recusada"), "{erro}");
}

#[test]
fn lista_com_acentos_espacos_tipos_e_permissoes() {
    let pasta = Pasta::nova("sftp-lista");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/relatório final.txt", b"abc");
    servidor.escrever("/home/pasta nova/dentro.txt", b"x");
    std::os::unix::fs::symlink("relatório final.txt", servidor.real("/home/atalho")).unwrap();
    let mut sftp = conectar(&servidor);

    let mut entradas = sftp.list("/home").unwrap();
    entradas.sort_by(|a, b| a.name.cmp(&b.name));
    let nomes: Vec<_> = entradas.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(nomes, ["atalho", "pasta nova", "relatório final.txt"]);

    assert_eq!(entradas[0].kind, EntryKind::Symlink);
    assert_eq!(
        entradas[0].link_target.as_deref(),
        Some("relatório final.txt")
    );
    assert_eq!(entradas[1].kind, EntryKind::Dir);
    assert_eq!(entradas[2].kind, EntryKind::File);
    assert_eq!(entradas[2].size, 3);
    assert!(entradas[2].permissions.is_some_and(|m| m & 0o600 == 0o600));
    assert!(entradas[2].modified.is_some());
    assert!(entradas[2].owner.is_some());

    let dentro = sftp.stat("/home/pasta nova/dentro.txt").unwrap();
    assert_eq!(dentro.name, "dentro.txt");
    assert_eq!(dentro.size, 1);
    let atalho = sftp.lstat("/home/atalho").unwrap();
    assert_eq!(atalho.kind, EntryKind::Symlink);
    assert_eq!(sftp.stat("/home/atalho").unwrap().kind, EntryKind::File);
}

#[test]
fn baixa_e_envia_5_mib_sem_perder_um_byte() {
    let pasta = Pasta::nova("sftp-5mib");
    let servidor = Servidor::novo(pasta.caminho());
    let conteudo = dados(5 * 1024 * 1024 + 123);
    servidor.escrever("/home/grande.bin", &conteudo);
    let mut sftp = conectar(&servidor);

    assert_eq!(ler_tudo(&mut sftp, "/home/grande.bin", 0), conteudo);

    gravar(&mut sftp, "/home/enviado.bin", 0, &conteudo);
    assert_eq!(
        std::fs::read(servidor.real("/home/enviado.bin")).unwrap(),
        conteudo
    );
}

#[test]
fn retoma_a_leitura_e_a_gravacao_de_um_deslocamento() {
    let pasta = Pasta::nova("sftp-retoma");
    let servidor = Servidor::novo(pasta.caminho());
    let conteudo = dados(300_000);
    servidor.escrever("/home/a.bin", &conteudo);
    let mut sftp = conectar(&servidor);

    assert_eq!(
        ler_tudo(&mut sftp, "/home/a.bin", 100_000),
        conteudo[100_000..]
    );

    servidor.escrever("/home/parcial.bin", &conteudo[..100_000]);
    gravar(
        &mut sftp,
        "/home/parcial.bin",
        100_000,
        &conteudo[100_000..],
    );
    assert_eq!(
        std::fs::read(servidor.real("/home/parcial.bin")).unwrap(),
        conteudo
    );
}

#[test]
fn renomeia_por_cima_so_com_permissao() {
    let pasta = Pasta::nova("sftp-renomeia");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/novo.txt", b"novo");
    servidor.escrever("/home/velho.txt", b"velho");
    let mut sftp = conectar(&servidor);

    let erro = sftp
        .rename("/home/novo.txt", "/home/velho.txt", false)
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(servidor.real("/home/velho.txt")).unwrap(),
        b"velho"
    );

    sftp.rename("/home/novo.txt", "/home/velho.txt", true)
        .unwrap();
    assert_eq!(
        std::fs::read(servidor.real("/home/velho.txt")).unwrap(),
        b"novo"
    );
    assert!(!servidor.real("/home/novo.txt").exists());
}

#[test]
fn pastas_criar_e_apagar_dizem_o_que_deu_errado() {
    let pasta = Pasta::nova("sftp-pastas");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/cheia/x.txt", b"x");
    let mut sftp = conectar(&servidor);

    sftp.mkdir("/home/vazia").unwrap();
    assert!(servidor.real("/home/vazia").is_dir());
    let existe = sftp.mkdir("/home/vazia").unwrap_err();
    assert_eq!(existe.kind(), io::ErrorKind::AlreadyExists, "{existe}");

    let cheia = sftp.remove_dir("/home/cheia").unwrap_err();
    assert_eq!(cheia.kind(), io::ErrorKind::DirectoryNotEmpty, "{cheia}");
    sftp.remove_dir("/home/vazia").unwrap();
    assert!(!servidor.real("/home/vazia").exists());
}

#[test]
fn erros_trazem_o_tipo_e_o_caminho_em_portugues() {
    let pasta = Pasta::nova("sftp-erros");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/secreto/x.txt", b"x");
    servidor.negar("/home/secreto");
    servidor.negar("/home/trancado.txt");
    servidor.escrever("/home/trancado.txt", b"x");
    let mut sftp = conectar(&servidor);

    let ausente = sftp.stat("/home/nao-existe").unwrap_err();
    assert_eq!(ausente.kind(), io::ErrorKind::NotFound);
    assert_eq!(ausente.to_string(), "não encontrado: /home/nao-existe");

    let negada = sftp.list("/home/secreto").unwrap_err();
    assert_eq!(negada.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(negada.to_string(), "sem permissão para ler /home/secreto");

    let trancado = sftp.open_read("/home/trancado.txt", 0).err().unwrap();
    assert_eq!(trancado.kind(), io::ErrorKind::PermissionDenied);

    let pasta_lida = sftp.open_read("/home", 0).err().unwrap();
    assert_eq!(pasta_lida.kind(), io::ErrorKind::IsADirectory);
}

#[test]
fn real_path_resolve_links_e_acusa_o_que_nao_existe() {
    let pasta = Pasta::nova("sftp-real-path");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/dentro/a.txt", b"a");
    let mut sftp = conectar(&servidor);
    sftp.symlink("dentro", "/home/atalho").unwrap();

    assert_eq!(
        sftp.real_path("/home/atalho/../atalho/a.txt").unwrap(),
        "/home/dentro/a.txt"
    );
    assert_eq!(
        sftp.real_path("/home/nada").unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn links_permissoes_e_data() {
    let pasta = Pasta::nova("sftp-meta");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/a.txt", b"a");
    let mut sftp = conectar(&servidor);

    sftp.symlink("a.txt", "/home/link").unwrap();
    assert_eq!(sftp.read_link("/home/link").unwrap(), "a.txt");
    assert_eq!(
        std::fs::read_link(servidor.real("/home/link")).unwrap(),
        std::path::Path::new("a.txt")
    );

    sftp.set_permissions("/home/a.txt", 0o640).unwrap();
    assert_eq!(sftp.stat("/home/a.txt").unwrap().permissions, Some(0o640));

    let data = UNIX_EPOCH + Duration::from_secs(1_400_000_000);
    sftp.set_modified("/home/a.txt", data).unwrap();
    assert_eq!(sftp.stat("/home/a.txt").unwrap().modified, Some(data));
}

#[test]
fn a_fila_envia_baixa_e_apaga_pastas() {
    let pasta = Pasta::nova("sftp-fila");
    let servidor = Servidor::novo(pasta.caminho());
    pasta.escrever("origem/um.txt", b"um");
    pasta.escrever("origem/sub/dois.txt", &dados(70_000));
    pasta.escrever("origem/sub/três com espaço.txt", b"tres");
    let fila = Transfers::new(3);
    let remoto = Endpoint::Remote(opener(&servidor));

    fila.add(
        (Endpoint::Local, pasta.texto("origem")),
        (remoto.clone(), "/home/destino".to_owned()),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(ESPERA));
    assert!(
        fila.items()
            .get_untracked()
            .iter()
            .all(|i| i.state == TransferState::Done)
    );
    assert_eq!(
        std::fs::read(servidor.real("/home/destino/sub/dois.txt")).unwrap(),
        dados(70_000)
    );
    assert!(
        servidor
            .real("/home/destino/sub/três com espaço.txt")
            .exists()
    );

    fila.add(
        (remoto.clone(), "/home/destino".to_owned()),
        (Endpoint::Local, pasta.texto("volta")),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(ESPERA));
    assert_eq!(
        std::fs::read(pasta.caminho().join("volta/sub/dois.txt")).unwrap(),
        dados(70_000)
    );

    fila.delete(remoto, "/home/destino");
    assert!(fila.wait_idle(ESPERA));
    assert!(!servidor.real("/home/destino").exists(), "apagou a árvore");
}

#[test]
fn a_remote_reabre_depois_que_o_cabo_e_puxado() {
    let pasta = Pasta::nova("sftp-reabre");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/a.txt", b"a");
    let remote = Remote::spawn("teste", opener(&servidor));
    let primeira = remote.list("/home");
    assert_eq!(esperar(&primeira).unwrap().len(), 1);
    assert_eq!(servidor.aceitas(), 1);

    servidor.derrubar();
    let segunda = remote.list("/home");
    assert_eq!(esperar(&segunda).unwrap().len(), 1, "o pedido foi repetido");
    assert_eq!(servidor.aceitas(), 2, "uma conexão nova");
    assert_eq!(remote.state().get_untracked(), RemoteState::Ready);
}

/// Um proxy SOCKS5 sem autenticação que atende uma conexão.
fn socks5() -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    let passou = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let contador = Arc::clone(&passou);
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut oferta = [0u8; 2];
        cliente.read_exact(&mut oferta).unwrap();
        let mut metodos = vec![0u8; usize::from(oferta[1])];
        cliente.read_exact(&mut metodos).unwrap();
        cliente.write_all(&[5, 0]).unwrap();
        let mut pedido = [0u8; 5];
        cliente.read_exact(&mut pedido).unwrap();
        let mut host = vec![0u8; usize::from(pedido[4])];
        cliente.read_exact(&mut host).unwrap();
        let mut porta = [0u8; 2];
        cliente.read_exact(&mut porta).unwrap();
        let destino = TcpStream::connect((
            String::from_utf8(host).unwrap().as_str(),
            u16::from_be_bytes(porta),
        ))
        .unwrap();
        contador.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        cliente.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        let (mut c1, mut d1) = (cliente.try_clone().unwrap(), destino.try_clone().unwrap());
        thread::spawn(move || {
            let _ = io::copy(&mut c1, &mut d1);
        });
        let (mut c2, mut d2) = (cliente, destino);
        let _ = io::copy(&mut d2, &mut c2);
    });
    (porta, passou)
}

#[test]
fn passa_pelo_proxy_socks5() {
    let pasta = Pasta::nova("sftp-proxy");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/a.txt", b"a");
    let (porta, passou) = socks5();
    let builder = servidor.builder().proxy(Proxy::Socks5 {
        host: "127.0.0.1".into(),
        port: porta,
        user: None,
        password: None,
    });
    let mut sftp = Sftp::connect(&builder).unwrap();
    assert_eq!(sftp.list("/home").unwrap().len(), 1);
    assert_eq!(passou.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// 64 MiB por um SFTP local: `cargo test -p tomni-ui-files --release -- --ignored`.
#[test]
#[ignore = "mede o desempenho; rode em release"]
fn desempenho_de_64_mib() {
    let pasta = Pasta::nova("sftp-desempenho");
    let servidor = Servidor::novo(pasta.caminho());
    let conteudo = dados(64 * 1024 * 1024);
    servidor.escrever("/home/grande.bin", &conteudo);
    let mut sftp = conectar(&servidor);

    let inicio = Instant::now();
    let lido = ler_tudo(&mut sftp, "/home/grande.bin", 0);
    let baixar = inicio.elapsed();
    assert_eq!(lido.len(), conteudo.len());

    let inicio = Instant::now();
    gravar(&mut sftp, "/home/enviado.bin", 0, &conteudo);
    let enviar = inicio.elapsed();
    println!("baixar {baixar:?}, enviar {enviar:?}");
    assert!(baixar < Duration::from_secs(4), "baixar levou {baixar:?}");
    assert!(enviar < Duration::from_secs(4), "enviar levou {enviar:?}");
}
