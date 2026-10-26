//! O SFTP aberto na conexão SSH de um terminal: sem outra conexão nem outra senha.
#![cfg(all(unix, feature = "sftp"))]

mod comum;

use std::io::{self, Read};
use std::time::{Duration, Instant};

use tomni_ui_files::Fs;
use tomni_ui_files::sftp::Sftp;
use tomni_ui_terminal::Session;
use tomni_ui_terminal::ssh::SshChannelOpener;

use comum::Pasta;
use comum::servidor::Servidor;

const ESPERA: Duration = Duration::from_secs(10);

/// Abre uma sessão de terminal no servidor e espera o shell responder.
fn sessao(servidor: &Servidor, chave: &str) -> Session {
    let sessao = Session::new(servidor.builder().connect()).key(chave);
    esperar(|| sessao.text().contains("bem-vinda, ana"));
    sessao
}

fn esperar(condicao: impl Fn() -> bool) {
    let limite = Instant::now() + ESPERA;
    loop {
        Session::poll_all();
        if condicao() {
            return;
        }
        assert!(Instant::now() < limite, "a condição não chegou a tempo");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn opener(sessao: &Session) -> SshChannelOpener {
    sessao
        .ssh_channels()
        .expect("a sessão SSH entrega o opener")
}

fn nomes(sftp: &mut Sftp, pasta: &str) -> Vec<String> {
    let mut nomes: Vec<String> = sftp
        .list(pasta)
        .unwrap()
        .into_iter()
        .map(|entrada| entrada.name)
        .collect();
    nomes.sort();
    nomes
}

#[test]
fn abre_o_sftp_na_conexao_do_terminal_sem_autenticar_de_novo() {
    let pasta = Pasta::nova("sessao-um");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/nota.txt", "oi".as_bytes());
    let sessao = sessao(&servidor, "um");

    let mut sftp = Sftp::from_channel(&opener(&sessao)).unwrap();
    let casa = sftp.home().unwrap();
    assert_eq!(nomes(&mut sftp, &casa), ["nota.txt"]);
    let mut texto = String::new();
    sftp.open_read("/home/nota.txt", 0)
        .unwrap()
        .read_to_string(&mut texto)
        .unwrap();
    assert_eq!(texto, "oi");

    assert_eq!(servidor.aceitas(), 1, "uma só conexão TCP");
    assert_eq!(servidor.autenticadas(), 1, "uma só autenticação");
    assert_eq!(servidor.sftps(), 1);
    // O terminal segue funcionando ao lado do SFTP.
    sessao.send("olá\r".as_bytes());
    esperar(|| sessao.text().contains("olá"));
    drop(sftp);
    sessao.dispose();
}

#[test]
fn varios_canais_na_mesma_conexao() {
    let pasta = Pasta::nova("sessao-varios");
    let servidor = Servidor::novo(pasta.caminho());
    let sessao = sessao(&servidor, "varios");
    let opener = opener(&sessao);

    let mut a = Sftp::from_channel(&opener).unwrap();
    let mut b = Sftp::from_channel(&opener).unwrap();
    a.mkdir("/home/de-a").unwrap();
    assert_eq!(nomes(&mut b, "/home"), ["de-a"]);
    assert_eq!(servidor.aceitas(), 1);
    assert_eq!(servidor.sftps(), 2);
    sessao.dispose();
}

#[test]
fn o_sftp_continua_depois_que_o_shell_sai_e_a_conexao_cai_com_o_ultimo_canal() {
    let pasta = Pasta::nova("sessao-sai");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/fica.txt", b"x");
    let sessao = sessao(&servidor, "sai");
    let opener = opener(&sessao);
    let mut sftp = Sftp::from_channel(&opener).unwrap();

    sessao.send("exit\r".as_bytes());
    esperar(|| sessao.closed());
    // A aba mostra o fim, mas o SFTP ainda anda.
    assert_eq!(nomes(&mut sftp, "/home"), ["fica.txt"]);

    drop(sftp);
    // Sem canais e sem shell, a conexão acaba: o opener deixa de abrir.
    let limite = Instant::now() + ESPERA;
    loop {
        match opener.open_subsystem("sftp", Duration::from_secs(2)) {
            Err(erro) if erro.kind() == io::ErrorKind::ConnectionReset => break,
            Err(erro) => panic!("erro inesperado: {erro}"),
            Ok(canal) => drop(canal),
        }
        assert!(Instant::now() < limite, "a conexão não acabou");
        std::thread::sleep(Duration::from_millis(20));
    }
    sessao.dispose();
}

#[test]
fn fechar_a_aba_derruba_o_sftp_e_o_opener_antigo_so_da_erro() {
    let pasta = Pasta::nova("sessao-fecha");
    let servidor = Servidor::novo(pasta.caminho());
    let sessao = sessao(&servidor, "fecha");
    let opener = opener(&sessao);
    let mut sftp = Sftp::from_channel(&opener).unwrap();
    assert!(sftp.home().is_ok());

    sessao.dispose();
    let limite = Instant::now() + ESPERA;
    while sftp.home().is_ok() {
        assert!(Instant::now() < limite, "o SFTP seguiu vivo");
        std::thread::sleep(Duration::from_millis(20));
    }
    let erro = sftp.home().unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::ConnectionReset, "{erro}");
    let erro = opener
        .open_subsystem("sftp", Duration::from_secs(2))
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::ConnectionReset, "{erro}");
}

#[test]
fn reiniciar_a_sessao_nao_leva_o_sftp_junto() {
    let pasta = Pasta::nova("sessao-reinicia");
    let servidor = Servidor::novo(pasta.caminho());
    let sessao = sessao(&servidor, "reinicia");
    let antigo = opener(&sessao);
    let mut sftp = Sftp::from_channel(&antigo).unwrap();

    sessao.send("exit\r".as_bytes());
    esperar(|| sessao.closed());
    assert!(sessao.restart());
    esperar(|| sessao.text().matches("bem-vinda, ana").count() >= 2);

    // O canal da conexão velha acaba; o opener novo abre outro na conexão nova.
    let limite = Instant::now() + ESPERA;
    while sftp.home().is_ok() {
        assert!(Instant::now() < limite, "o SFTP antigo seguiu vivo");
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut novo = Sftp::from_channel(&opener(&sessao)).unwrap();
    assert!(novo.home().is_ok());
    assert_eq!(servidor.aceitas(), 2);
    sessao.dispose();
}

#[test]
fn sessao_que_nao_e_ssh_nao_entrega_opener() {
    let sessao = Session::new(tomni_ui_terminal::Loopback::new());
    assert!(sessao.ssh_channels().is_none());
    sessao.dispose();
}
