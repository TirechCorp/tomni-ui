//! O teste real da sonda de pasta (M64): `SshChannelOpener::shell_dir` contra o `sshd` de verdade,
//! em Docker. Fora do CI. Rode com `testes/pasta/rodar.sh`, que sobe os servidores
//! (`testes/pasta/docker-compose.yml`), roda estes testes (`#[ignore]`) e derruba.
//!
//! Os servidores, nas portas do próprio computador: o OpenSSH atual do Alpine (22201, com o
//! `sshd-session`), o OpenSSH 9.2 do Debian (22202) e o Dropbear (22203). Cada um tem os usuários
//! `teste` (shell `bash`) e `peixe` (shell `fish`), com a senha igual ao nome e `sudo` sem senha.

use std::thread;
use std::time::{Duration, Instant};

use tomni_ui_reactive::untrack;
use tomni_ui_terminal::{Session, Ssh};

const SERVIDORES: [(&str, u16); 3] = [
    ("OpenSSH novo (Alpine)", 22201),
    ("OpenSSH antigo (Debian 12)", 22202),
    ("Dropbear (Alpine)", 22203),
];

/// Bombeia as sessões até `cond` valer, com limite de `segundos`.
fn esperar(segundos: u64, mut cond: impl FnMut() -> bool) -> bool {
    let limite = Instant::now() + Duration::from_secs(segundos);
    loop {
        Session::poll_all();
        if untrack(&mut cond) {
            return true;
        }
        if Instant::now() > limite {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn abrir(porta: u16, usuario: &str) -> Session {
    let ssh = Ssh::builder("127.0.0.1", porta, usuario)
        .password(usuario)
        .check_host_key(|_, _| true)
        .connect();
    Session::new(ssh)
}

/// Digita `linha`, dá Enter e espera o prompt voltar (uma marca que o eco não imita, porque o
/// eco leva as aspas).
fn digitar(sessao: &Session, linha: &str, n: &mut u32) {
    *n += 1;
    let marca = format!("MARCA{}", *n + 1000);
    sessao.send(format!("{linha}\r").as_bytes());
    sessao.send(format!("echo MARCA\"\"{}\r", *n + 1000).as_bytes());
    assert!(
        esperar(20, || sessao.text().lines().any(|l| l.trim_end() == marca)),
        "sem {marca} depois de `{linha}`:\n{}",
        sessao.text()
    );
}

/// Pede a pasta até ela ser `esperada` (a tela e o servidor levam um instante para acertar).
fn pasta_ate(sessao: &Session, esperada: &str) -> Option<String> {
    let canais = sessao.ssh_channels().expect("a sessão é SSH");
    let mut ultima = None;
    esperar(10, || {
        ultima = canais
            .shell_dir(Duration::from_secs(5))
            .expect("o canal exec abre");
        ultima.as_deref() == Some(esperada)
    });
    ultima
}

fn roda(nome: &str, porta: u16, usuario: &str) {
    eprintln!("== {nome}, usuário {usuario}");
    let sessao = abrir(porta, usuario);
    assert!(
        esperar(30, || !sessao.connecting()
            && !sessao.text().trim().is_empty()),
        "a conexão não abriu: {}",
        sessao.text()
    );
    let mut n = 0;
    digitar(&sessao, "cd /etc", &mut n);
    assert_eq!(pasta_ate(&sessao, "/etc").as_deref(), Some("/etc"));

    // O processo em primeiro plano manda: um `sleep` que começou em outra pasta.
    sessao.send(b"sh -c 'cd /var && exec sleep 60'\r");
    assert_eq!(pasta_ate(&sessao, "/var").as_deref(), Some("/var"));
    sessao.send(b"\x03");
    digitar(&sessao, "echo volta", &mut n);
    assert_eq!(pasta_ate(&sessao, "/etc").as_deref(), Some("/etc"));

    // `sudo -i`: o primeiro plano é de outro dono, e a pasta vem do shell de antes.
    sessao.send(b"sudo -i\r");
    digitar(&sessao, "cd /root", &mut n);
    assert_eq!(
        pasta_ate(&sessao, "/etc").as_deref(),
        Some("/etc"),
        "depois do sudo -i"
    );
    sessao.send(b"exit\r");
    sessao.dispose();
}

#[test]
#[ignore = "precisa dos servidores de testes/pasta (Docker)"]
fn a_sonda_acha_a_pasta_nos_tres_servidores_com_bash() {
    for (nome, porta) in SERVIDORES {
        roda(nome, porta, "teste");
    }
}

#[test]
#[ignore = "precisa dos servidores de testes/pasta (Docker)"]
fn a_sonda_acha_a_pasta_nos_tres_servidores_com_fish() {
    for (nome, porta) in SERVIDORES {
        roda(nome, porta, "peixe");
    }
}
