//! Testes de X11 e túneis, contra o servidor `russh` de teste do módulo pai.

use super::*;
use std::sync::atomic::AtomicU16;

/// Um servidor de eco TCP: devolve tudo o que recebe.
fn eco() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        while let Ok((conexao, _)) = listener.accept() {
            let mut leitura = conexao.try_clone().unwrap();
            let mut escrita = conexao;
            thread::spawn(move || {
                let _ = std::io::copy(&mut leitura, &mut escrita);
            });
        }
    });
    porta
}

/// Uma porta TCP que ninguém escuta.
fn porta_livre() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Espera o túnel `indice` escutar e devolve o endereço onde ele escuta.
fn escutando(forwards: &SshForwards, indice: usize) -> String {
    esperar(|| forwards.list()[indice].listening.is_some());
    forwards.list()[indice]
        .listening
        .clone()
        .unwrap_or_else(|| panic!("o túnel não subiu: {:?}", forwards.list()))
}

/// Conecta em `endereco` (`127.0.0.1:porta`), manda `texto` e lê o eco.
fn ecoar(endereco: &str, texto: &[u8]) -> TcpStream {
    let mut conexao = TcpStream::connect(endereco).unwrap();
    conexao
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    conexao.write_all(texto).unwrap();
    let mut volta = vec![0u8; texto.len()];
    conexao.read_exact(&mut volta).unwrap();
    assert_eq!(volta, texto);
    conexao
}

fn conecta(teste: &Teste, forward: Forward) -> (Ssh, SshForwards) {
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").forward(forward).connect();
    ler_ate(&mut ssh, "bem-vinda");
    let forwards = ssh.ssh_forwards().expect("o Ssh tem túneis");
    (ssh, forwards)
}

// ----- -L -----

#[test]
fn o_tunel_local_leva_o_eco_e_conta_os_bytes() {
    let teste = Teste::novo("local");
    let alvo = eco();
    let (_ssh, forwards) = conecta(&teste, Forward::local(0, "127.0.0.1", alvo));
    let endereco = escutando(&forwards, 0);
    assert!(endereco.starts_with("127.0.0.1:"), "{endereco}");

    let conexao = ecoar(&endereco, b"ola, tunel");
    esperar(|| forwards.list()[0].active == 1);
    drop(conexao);
    esperar(|| forwards.list()[0].active == 0);

    let estado = &forwards.list()[0];
    assert_eq!(estado.state, ForwardState::Listening);
    assert_eq!((estado.active, estado.total), (0, 1));
    assert_eq!((estado.sent, estado.received), (10, 10));
    assert!(teste.log().contains(&format!("túnel 127.0.0.1:{alvo}")));
}

#[test]
fn o_tunel_local_so_escuta_no_loopback_a_nao_ser_que_abra_para_a_rede() {
    let teste = Teste::novo("local-rede");
    let alvo = eco();
    let (_ssh, forwards) = conecta(
        &teste,
        Forward::local(0, "127.0.0.1", alvo).open_to_network(),
    );
    let endereco = escutando(&forwards, 0);
    assert!(endereco.starts_with("0.0.0.0:"), "{endereco}");
}

#[test]
fn o_servidor_que_proibe_tuneis_aparece_no_estado_do_tunel() {
    let teste = Teste::novo("local-proibido");
    teste.modo.bloqueia_tuneis.store(true, Ordering::SeqCst);
    let alvo = eco();
    let (mut ssh, forwards) = conecta(&teste, Forward::local(0, "127.0.0.1", alvo));
    let endereco = escutando(&forwards, 0);
    let mut conexao = TcpStream::connect(&endereco).unwrap();
    conexao
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut resto = Vec::new();
    // A conexão local fecha, sem dados, porque o servidor não abriu o canal.
    let _ = conexao.read_to_end(&mut resto);
    assert!(resto.is_empty());
    esperar(|| forwards.list()[0].last_error.is_some());
    let erro = forwards.list()[0].last_error.clone().unwrap();
    assert!(erro.contains("proíbe túneis"), "{erro}");
    // Uma conexão recusada não enche o terminal: o motivo fica no estado do túnel.
    let mut lido = Vec::new();
    ssh.read(&mut lido, usize::MAX);
    assert!(!String::from_utf8_lossy(&lido).contains("proíbe"));
}

#[test]
fn porta_ocupada_falha_avisa_e_o_restart_sobe_depois_de_liberar() {
    let teste = Teste::novo("ocupada");
    teste.conhecer_servidor();
    let ocupada = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = ocupada.local_addr().unwrap().port();
    let alvo = eco();
    let mut ssh = teste
        .builder()
        .password("senha")
        .forward(Forward::local(porta, "127.0.0.1", alvo))
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "já está em uso");
    assert_eq!(status, Status::Open, "{texto}");
    assert!(texto.contains("já está em uso"), "{texto}");
    let forwards = ssh.ssh_forwards().unwrap();
    let ForwardState::Failed(motivo) = forwards.list()[0].state.clone() else {
        panic!("devia falhar: {:?}", forwards.list());
    };
    assert!(motivo.contains("já está em uso"), "{motivo}");

    drop(ocupada);
    forwards.restart(forwards.list()[0].id);
    let endereco = escutando(&forwards, 0);
    assert_eq!(endereco, format!("127.0.0.1:{porta}"));
    ecoar(&endereco, b"oi");
}

#[test]
fn exit_on_forward_failure_fecha_a_conexao_com_o_motivo() {
    let teste = Teste::novo("ocupada-fecha");
    teste.conhecer_servidor();
    let ocupada = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = ocupada.local_addr().unwrap().port();
    let mut ssh = teste
        .builder()
        .password("senha")
        .forward(Forward::local(porta, "127.0.0.1", eco()))
        .exit_on_forward_failure(true)
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    let Status::Closed(Some(mensagem)) = status else {
        panic!("devia fechar: {status:?}");
    };
    assert!(mensagem.contains("já está em uso"), "{mensagem}");
}

#[test]
fn stop_para_de_escutar_e_restart_volta() {
    let teste = Teste::novo("stop");
    let (_ssh, forwards) = conecta(&teste, Forward::local(0, "127.0.0.1", eco()));
    let primeiro = escutando(&forwards, 0);
    let id = forwards.list()[0].id;

    forwards.stop(id);
    esperar(|| forwards.list()[0].state == ForwardState::Stopped);
    assert_eq!(forwards.list()[0].listening, None);
    esperar(|| TcpStream::connect(&primeiro).is_err());
    assert!(TcpStream::connect(&primeiro).is_err());

    forwards.restart(id);
    let segundo = escutando(&forwards, 0);
    ecoar(&segundo, b"de novo");
}

#[test]
fn add_sobe_um_tunel_novo_na_conexao_aberta() {
    let teste = Teste::novo("add");
    let (_ssh, forwards) = conecta(&teste, Forward::local(0, "127.0.0.1", eco()));
    assert_eq!(forwards.list().len(), 1);
    let id = forwards.add(Forward::local(0, "127.0.0.1", eco()));
    esperar(|| forwards.list().len() == 2 && forwards.list()[1].listening.is_some());
    assert_eq!(forwards.list()[1].id, id);
    ecoar(&escutando(&forwards, 1), b"segundo");
}

// ----- -D -----

/// Fala o SOCKS5 sem autenticação com `endereco` e devolve o código da resposta ao CONNECT.
fn socks_connect(conexao: &mut TcpStream, destino: &[u8]) -> u8 {
    conexao.write_all(&[5, 1, 0]).unwrap();
    let mut escolha = [0u8; 2];
    conexao.read_exact(&mut escolha).unwrap();
    assert_eq!(escolha, [5, 0]);
    conexao.write_all(destino).unwrap();
    let mut resposta = [0u8; 10];
    conexao.read_exact(&mut resposta).unwrap();
    assert_eq!(resposta[0], 5);
    resposta[1]
}

#[test]
fn o_tunel_dinamico_fala_socks5_com_ip_e_com_nome() {
    let teste = Teste::novo("socks");
    let alvo = eco();
    let (_ssh, forwards) = conecta(&teste, Forward::dynamic(0));
    let endereco = escutando(&forwards, 0);

    let mut ip = TcpStream::connect(&endereco).unwrap();
    ip.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut pedido = vec![5, 1, 0, 1, 127, 0, 0, 1];
    pedido.extend_from_slice(&alvo.to_be_bytes());
    assert_eq!(socks_connect(&mut ip, &pedido), 0);
    ip.write_all(b"por ip").unwrap();
    let mut volta = [0u8; 6];
    ip.read_exact(&mut volta).unwrap();
    assert_eq!(&volta, b"por ip");

    let mut nome = TcpStream::connect(&endereco).unwrap();
    nome.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let host = b"127.0.0.1";
    let mut pedido = vec![5, 1, 0, 3, host.len() as u8];
    pedido.extend_from_slice(host);
    pedido.extend_from_slice(&alvo.to_be_bytes());
    // A resposta de nome tem o endereço zerado, de 10 bytes também.
    assert_eq!(socks_connect(&mut nome, &pedido), 0);
    nome.write_all(b"por nome").unwrap();
    let mut volta = [0u8; 8];
    nome.read_exact(&mut volta).unwrap();
    assert_eq!(&volta, b"por nome");

    assert!(teste.log().contains(&format!("túnel 127.0.0.1:{alvo}")));
    esperar(|| forwards.list()[0].total == 2);
    assert_eq!(forwards.list()[0].total, 2);
}

#[test]
fn o_tunel_dinamico_responde_recusado_quando_o_destino_nao_abre() {
    let teste = Teste::novo("socks-recusado");
    let fechada = porta_livre();
    let (_ssh, forwards) = conecta(&teste, Forward::dynamic(0));
    let endereco = escutando(&forwards, 0);
    let mut conexao = TcpStream::connect(&endereco).unwrap();
    conexao
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut pedido = vec![5, 1, 0, 1, 127, 0, 0, 1];
    pedido.extend_from_slice(&fechada.to_be_bytes());
    assert_eq!(socks_connect(&mut conexao, &pedido), 5);
}

// ----- -R -----

#[test]
fn o_tunel_remoto_pede_a_porta_zero_e_leva_o_eco_para_este_lado() {
    let teste = Teste::novo("remoto");
    let alvo = eco();
    let (_ssh, forwards) = conecta(&teste, Forward::remote(0, "127.0.0.1", alvo));
    let escuta = escutando(&forwards, 0);
    let porta: u16 = escuta.rsplit(':').next().unwrap().parse().unwrap();
    assert_ne!(porta, 0);

    ecoar(&format!("127.0.0.1:{porta}"), b"de la para ca");
    esperar(|| forwards.list()[0].total == 1);
    assert_eq!(forwards.list()[0].total, 1);

    forwards.stop(forwards.list()[0].id);
    esperar(|| forwards.list()[0].state == ForwardState::Stopped);
    esperar(|| {
        teste
            .log()
            .iter()
            .any(|linha| linha.starts_with("cancel-tcpip-forward"))
    });
    assert!(
        teste
            .log()
            .contains(&format!("cancel-tcpip-forward localhost:{porta}")),
        "{:?}",
        teste.log()
    );
}

#[test]
fn o_tunel_remoto_recusado_pelo_servidor_falha_com_o_motivo() {
    let teste = Teste::novo("remoto-proibido");
    teste.modo.bloqueia_tuneis.store(true, Ordering::SeqCst);
    teste.conhecer_servidor();
    let mut ssh = teste
        .builder()
        .password("senha")
        .forward(Forward::remote(0, "127.0.0.1", eco()))
        .connect();
    let (texto, _) = ler_ate(&mut ssh, "AllowTcpForwarding");
    assert!(texto.contains("AllowTcpForwarding"), "{texto}");
    let forwards = ssh.ssh_forwards().unwrap();
    assert!(matches!(forwards.list()[0].state, ForwardState::Failed(_)));
}

// ----- socket Unix -----

#[cfg(unix)]
#[test]
fn o_tunel_de_socket_unix_leva_o_eco_com_permissao_0600() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let teste = Teste::novo("socket");
    let remoto = teste.pasta.join("remoto.sock");
    let servidor = UnixListener::bind(&remoto).unwrap();
    thread::spawn(move || {
        while let Ok((mut conexao, _)) = servidor.accept() {
            thread::spawn(move || {
                let mut leitura = conexao.try_clone().unwrap();
                let _ = std::io::copy(&mut leitura, &mut conexao);
            });
        }
    });
    let local = teste.pasta.join("local.sock");
    let (_ssh, forwards) = conecta(
        &teste,
        Forward::unix(
            LocalSocket::Path(local.clone()),
            remoto.to_string_lossy().into_owned(),
        ),
    );
    escutando(&forwards, 0);
    assert_eq!(
        std::fs::metadata(&local).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let mut conexao = std::os::unix::net::UnixStream::connect(&local).unwrap();
    conexao
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    conexao.write_all(b"pelo socket").unwrap();
    let mut volta = [0u8; 11];
    conexao.read_exact(&mut volta).unwrap();
    assert_eq!(&volta, b"pelo socket");
    assert!(
        teste
            .log()
            .contains(&format!("socket {}", remoto.display()))
    );

    forwards.stop(forwards.list()[0].id);
    esperar(|| !local.exists());
    assert!(!local.exists(), "o socket devia ser apagado ao parar");
}

// ----- salto, fim da conexão, no_shell e command -----

#[test]
fn o_tunel_passa_pelo_salto() {
    let (salto, destino) = (Teste::novo("tunel-salto"), Teste::novo("tunel-destino"));
    for t in [&salto, &destino] {
        t.conhecer_servidor();
    }
    let alvo = eco();
    let mut ssh = destino
        .builder()
        .password("senha")
        .jump(salto.builder().password("senha"))
        .forward(Forward::local(0, "127.0.0.1", alvo))
        .connect();
    ler_ate(&mut ssh, "bem-vinda");
    let forwards = ssh.ssh_forwards().unwrap();
    ecoar(&escutando(&forwards, 0), b"via salto");
    // O salto só abriu o caminho até o destino; o túnel de dados é do destino.
    assert_eq!(salto.log(), [format!("túnel 127.0.0.1:{}", destino.porta)]);
    assert!(destino.log().contains(&format!("túnel 127.0.0.1:{alvo}")));
}

#[test]
fn com_uma_conexao_de_tunel_aberta_a_sessao_espera_o_fim_dela() {
    let teste = Teste::novo("espera");
    let (mut ssh, forwards) = conecta(&teste, Forward::local(0, "127.0.0.1", eco()));
    let mut conexao = ecoar(&escutando(&forwards, 0), b"segura");
    esperar(|| forwards.list()[0].active == 1);

    // O terminal fecha com o código do shell, mas a conexão segue de pé para o túnel.
    ssh.write(b"exit 3\r").unwrap();
    let (texto, status) = ler_ate(&mut ssh, "esperando 1 conexão dos túneis");
    assert!(texto.contains("esperando 1 conexão dos túneis"), "{texto}");
    assert!(matches!(status, Status::Closed(Some(_))), "{status:?}");
    conexao.write_all(b"ainda").unwrap();
    let mut volta = [0u8; 5];
    conexao.read_exact(&mut volta).unwrap();
    assert_eq!(&volta, b"ainda");

    // Com o túnel livre, a conexão acaba (o servidor vê o fim dela).
    drop(conexao);
    esperar(|| forwards.list()[0].active == 0);
    esperar(|| forwards.list()[0].state == ForwardState::Stopped);
    assert_eq!(forwards.list()[0].state, ForwardState::Stopped);
}

#[test]
fn no_shell_so_abre_os_tuneis_e_o_close_encerra() {
    let teste = Teste::novo("sem-shell");
    teste.conhecer_servidor();
    let alvo = eco();
    let mut ssh = teste
        .builder()
        .password("senha")
        .no_shell(true)
        .forward(Forward::local(0, "127.0.0.1", alvo))
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "Túnel local");
    assert_eq!(status, Status::Open, "{texto}");
    let forwards = ssh.ssh_forwards().unwrap();
    ecoar(&escutando(&forwards, 0), b"sem shell");
    assert!(
        !teste
            .log()
            .iter()
            .any(|linha| linha.starts_with("pty") || linha == "shell"),
        "{:?}",
        teste.log()
    );

    ssh.commands.send(Command::Close).unwrap();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert!(matches!(status, Status::Closed(_)), "{status:?}");
}

#[test]
fn command_roda_o_comando_no_lugar_do_shell() {
    let teste = Teste::novo("command");
    teste.conhecer_servidor();
    let mut ssh = teste
        .builder()
        .password("senha")
        .command("virt-manager --no-fork")
        .connect();
    ler_ate(&mut ssh, "\u{0}");
    esperar(|| teste.log().iter().any(|l| l.starts_with("exec-pty")));
    assert!(
        teste
            .log()
            .contains(&"exec-pty virt-manager --no-fork".to_owned()),
        "{:?}",
        teste.log()
    );
    assert!(!teste.log().contains(&"shell".to_owned()));
}

// ----- X11 -----

/// O número de um display livre (a porta 6000+N de `127.0.0.1` livre).
fn display_livre() -> u16 {
    static PROXIMO: AtomicU16 = AtomicU16::new(0);
    loop {
        let numero =
            60 + (PROXIMO.fetch_add(1, Ordering::SeqCst) + std::process::id() as u16) % 500;
        if TcpListener::bind(("127.0.0.1", 6000 + numero)).is_ok() {
            return numero;
        }
    }
}

/// O que o servidor X de mentira recebeu.
#[derive(Clone, Default)]
struct Recebido(Arc<Mutex<Vec<u8>>>);

/// Um servidor X de mentira no display `numero`: lê o setup, guarda os bytes, responde "ok" e
/// depois devolve o que receber.
fn servidor_x(numero: u16) -> Recebido {
    let listener = TcpListener::bind(("127.0.0.1", 6000 + numero)).unwrap();
    let recebido = Recebido::default();
    let guarda = recebido.clone();
    thread::spawn(move || {
        while let Ok((mut conexao, _)) = listener.accept() {
            let guarda = guarda.clone();
            thread::spawn(move || {
                let mut cabecalho = [0u8; 12];
                if conexao.read_exact(&mut cabecalho).is_err() {
                    return;
                }
                let palavra = |de: usize| {
                    let par = [cabecalho[de], cabecalho[de + 1]];
                    usize::from(if cabecalho[0] == b'B' {
                        u16::from_be_bytes(par)
                    } else {
                        u16::from_le_bytes(par)
                    })
                };
                let resto = palavra(6).next_multiple_of(4) + palavra(8).next_multiple_of(4);
                let mut pacote = cabecalho.to_vec();
                pacote.resize(12 + resto, 0);
                if conexao.read_exact(&mut pacote[12..]).is_err() {
                    return;
                }
                *guarda.0.lock().unwrap() = pacote;
                let _ = conexao.write_all(b"ok");
                let mut leitura = conexao.try_clone().unwrap();
                let _ = std::io::copy(&mut leitura, &mut conexao);
            });
        }
    });
    recebido
}

/// Um `Xauthority` com uma entrada, no formato do `xauth`.
fn xauthority(familia: u16, endereco: &[u8], numero: u16, cookie: &[u8]) -> Vec<u8> {
    let mut bytes = familia.to_be_bytes().to_vec();
    for campo in [
        endereco,
        numero.to_string().as_bytes(),
        b"MIT-MAGIC-COOKIE-1",
        cookie,
    ] {
        bytes.extend_from_slice(&(campo.len() as u16).to_be_bytes());
        bytes.extend_from_slice(campo);
    }
    bytes
}

fn cookie_real() -> Vec<u8> {
    (1..=16).map(|b| b * 3).collect()
}

fn contem(bytes: &[u8], trecho: &[u8]) -> bool {
    bytes.windows(trecho.len()).any(|janela| janela == trecho)
}

fn escrever_xauthority(teste: &Teste, entrada: Vec<u8>) -> PathBuf {
    let arquivo = teste.pasta.join("Xauthority");
    std::fs::write(&arquivo, entrada).unwrap();
    arquivo
}

fn conecta_x11(teste: &Teste, x11: X11) -> Ssh {
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").x11(x11).connect();
    ler_ate(&mut ssh, "bem-vinda");
    ssh
}

#[test]
fn o_x11_troca_o_cookie_falso_pelo_real_e_leva_os_dados() {
    let teste = Teste::novo("x11");
    let numero = display_livre();
    let recebido = servidor_x(numero);
    let arquivo = escrever_xauthority(
        &teste,
        xauthority(0, &[127, 0, 0, 1], numero, &cookie_real()),
    );
    let mut ssh = conecta_x11(
        &teste,
        X11::trusted()
            .display(format!("127.0.0.1:{numero}"))
            .auth_file(arquivo),
    );
    assert!(
        teste
            .log()
            .iter()
            .any(|l| l.starts_with("x11 MIT-MAGIC-COOKIE-1 ") && l.ends_with(" 0 false")),
        "{:?}",
        teste.log()
    );

    ssh.write(b"x11\r").unwrap();
    esperar(|| teste.log().iter().any(|l| l.starts_with("x11 ok")));
    assert!(
        teste.log().contains(&"x11 ok ok".to_owned()),
        "{:?}",
        teste.log()
    );
    let pacote = recebido.0.lock().unwrap().clone();
    assert!(contem(&pacote, &cookie_real()), "{pacote:?}");
    assert!(!contem(&pacote, &[0u8; 16]), "o cookie falso vazou");

    let estado = ssh
        .ssh_forwards()
        .unwrap()
        .x11()
        .expect("o X11 está ligado");
    assert_eq!(estado.mode, X11Mode::Trusted);
    assert_eq!(estado.total, 1);
    assert_eq!(estado.error, None);
}

#[test]
fn o_x11_aceita_a_entrada_wild_do_xauthority() {
    let teste = Teste::novo("x11-wild");
    let numero = display_livre();
    let recebido = servidor_x(numero);
    let arquivo = escrever_xauthority(&teste, xauthority(65535, b"", numero, &cookie_real()));
    let mut ssh = conecta_x11(
        &teste,
        X11::trusted()
            .display(format!("127.0.0.1:{numero}"))
            .auth_file(arquivo),
    );
    ssh.write(b"x11\r").unwrap();
    esperar(|| teste.log().iter().any(|l| l.starts_with("x11 ok")));
    assert!(contem(&recebido.0.lock().unwrap(), &cookie_real()));
}

#[test]
fn o_x11_com_o_cookie_errado_fecha_o_canal_sem_tocar_no_display() {
    let teste = Teste::novo("x11-errado");
    let numero = display_livre();
    let recebido = servidor_x(numero);
    let arquivo = escrever_xauthority(
        &teste,
        xauthority(0, &[127, 0, 0, 1], numero, &cookie_real()),
    );
    let mut ssh = conecta_x11(
        &teste,
        X11::trusted()
            .display(format!("127.0.0.1:{numero}"))
            .auth_file(arquivo),
    );
    ssh.write(b"x11 errado\r").unwrap();
    esperar(|| teste.log().iter().any(|l| l.starts_with("x11 fechado")));
    assert!(
        teste.log().contains(&"x11 fechado".to_owned()),
        "{:?}",
        teste.log()
    );
    assert!(recebido.0.lock().unwrap().is_empty());
}

#[test]
fn o_x11_sem_entrada_no_xauthority_segue_sem_autenticacao() {
    let teste = Teste::novo("x11-sem-cookie");
    let numero = display_livre();
    let recebido = servidor_x(numero);
    let mut ssh = conecta_x11(
        &teste,
        X11::trusted()
            .display(format!("127.0.0.1:{numero}"))
            .auth_file(teste.pasta.join("nao-existe")),
    );
    ssh.write(b"x11\r").unwrap();
    esperar(|| teste.log().iter().any(|l| l.starts_with("x11 ok")));
    let pacote = recebido.0.lock().unwrap().clone();
    assert_eq!(pacote.len(), 12, "{pacote:?}");
    assert_eq!(&pacote[6..10], &[0, 0, 0, 0], "{pacote:?}");
}

#[test]
fn sem_servidor_x_o_canal_e_recusado_e_o_aviso_vem_uma_vez() {
    let teste = Teste::novo("x11-sem-servidor");
    let numero = display_livre();
    let mut ssh = conecta_x11(
        &teste,
        X11::trusted().display(format!("127.0.0.1:{numero}")),
    );
    ssh.write(b"x11\r").unwrap();
    let (texto, _) = ler_ate(&mut ssh, "não há servidor X");
    assert!(texto.contains("não há servidor X"), "{texto}");
    esperar(|| teste.log().contains(&"x11 recusado".to_owned()));
    assert!(teste.log().contains(&"x11 recusado".to_owned()));

    ssh.write(b"x11\r").unwrap();
    esperar(|| teste.log().iter().filter(|l| *l == "x11 recusado").count() == 2);
    let mut resto = Vec::new();
    thread::sleep(Duration::from_millis(200));
    ssh.read(&mut resto, usize::MAX);
    assert!(
        !String::from_utf8_lossy(&resto).contains("não há servidor X"),
        "o aviso devia vir uma vez só"
    );
    let estado = ssh.ssh_forwards().unwrap().x11().unwrap();
    assert!(estado.error.unwrap().contains("não há servidor X"));
}

#[test]
fn o_servidor_que_recusa_o_x11_gera_um_aviso_e_a_conexao_segue() {
    let teste = Teste::novo("x11-recusado");
    teste.modo.recusa_x11.store(true, Ordering::SeqCst);
    teste.conhecer_servidor();
    let mut ssh = teste
        .builder()
        .password("senha")
        .x11(X11::trusted().display("127.0.0.1:99"))
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    assert!(texto.contains("recusou o encaminhamento do X11"), "{texto}");
}

#[test]
fn x11_sem_display_avisa_e_segue_sem_x11() {
    let teste = Teste::novo("x11-sem-display");
    teste.conhecer_servidor();
    let mut ssh = teste
        .builder()
        .password("senha")
        .x11(X11::trusted().display("::formato errado"))
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    assert!(texto.contains("Encaminhamento X11 desligado"), "{texto}");
    assert!(ssh.ssh_forwards().unwrap().x11().is_none());
    assert!(!teste.log().iter().any(|l| l.starts_with("x11")));
}

#[test]
fn o_x11_nao_confiavel_sem_xauth_cai_para_o_confiavel_e_avisa() {
    let teste = Teste::novo("x11-sem-xauth");
    let numero = display_livre();
    let ssh = {
        teste.conhecer_servidor();
        let mut ssh = teste
            .builder()
            .password("senha")
            .x11(
                X11::untrusted()
                    .display(format!("127.0.0.1:{numero}"))
                    .auth_file(teste.pasta.join("nao-existe"))
                    .xauth_program("/nao/existe"),
            )
            .connect();
        let (texto, _) = ler_ate(&mut ssh, "bem-vinda");
        assert!(texto.contains("não confiável indisponível"), "{texto}");
        ssh
    };
    let estado = ssh.ssh_forwards().unwrap().x11().unwrap();
    assert_eq!(estado.mode, X11Mode::Trusted);
    drop(ssh);
}

#[cfg(unix)]
#[test]
fn o_x11_nao_confiavel_usa_o_cookie_que_o_xauth_gerou() {
    use std::os::unix::fs::PermissionsExt;

    let teste = Teste::novo("x11-xauth");
    let numero = display_livre();
    let recebido = servidor_x(numero);
    // Um `xauth` de mentira: grava no arquivo de `-f` uma entrada com o cookie "UUUU…".
    let programa = teste.pasta.join("xauth-falso");
    let entrada = format!(
        "#!/bin/sh\nprintf '\\000\\000\\000\\011127.0.0.1\\000\\00{}{}\\000\\022MIT-MAGIC-COOKIE-1\\000\\020UUUUUUUUUUUUUUUU' > \"$2\"\n",
        numero.to_string().len(),
        numero
    );
    std::fs::write(&programa, entrada).unwrap();
    std::fs::set_permissions(&programa, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut ssh = conecta_x11(
        &teste,
        X11::untrusted()
            .display(format!("127.0.0.1:{numero}"))
            .auth_file(teste.pasta.join("nao-existe"))
            .xauth_program(&programa),
    );
    ssh.write(b"x11\r").unwrap();
    esperar(|| teste.log().iter().any(|l| l.starts_with("x11 ok")));
    let pacote = recebido.0.lock().unwrap().clone();
    assert!(contem(&pacote, &[b'U'; 16]), "{pacote:?}");
    assert_eq!(
        ssh.ssh_forwards().unwrap().x11().unwrap().mode,
        X11Mode::Untrusted
    );
}

#[test]
fn o_salto_nao_recebe_o_pedido_de_x11() {
    let (salto, destino) = (Teste::novo("x11-salto"), Teste::novo("x11-destino"));
    for t in [&salto, &destino] {
        t.conhecer_servidor();
    }
    let numero = display_livre();
    let mut ssh = destino
        .builder()
        .password("senha")
        .jump(salto.builder().password("senha"))
        .x11(X11::trusted().display(format!("127.0.0.1:{numero}")))
        .connect();
    ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(salto.log(), [format!("túnel 127.0.0.1:{}", destino.porta)]);
    assert!(destino.log().iter().any(|l| l.starts_with("x11 ")));
}

// ----- launch -----

/// Espera o `launch` sair do "conectando"/"rodando" e devolve o estado final.
fn esperar_o_fim(app: &RemoteApp) -> RemoteAppStatus {
    esperar(|| {
        !matches!(
            app.status(),
            RemoteAppStatus::Connecting | RemoteAppStatus::Running
        )
    });
    app.status()
}

#[test]
fn launch_roda_o_comando_sem_terminal_e_acaba_com_zero() {
    let teste = Teste::novo("launch");
    teste.conhecer_servidor();
    teste.modo.exec_imediato.store(true, Ordering::SeqCst);
    let app = teste
        .builder()
        .password("senha")
        .launch("echo aviso >&2; exit 0");
    assert_eq!(esperar_o_fim(&app), RemoteAppStatus::Exited(Some(0)));
    assert!(app.stderr_tail().contains("aviso"), "{}", app.stderr_tail());
    let log = teste.log();
    assert!(!log.iter().any(|l| l.starts_with("pty")), "{log:?}");
    assert!(!log.contains(&"shell".to_owned()), "{log:?}");
    assert!(
        log.contains(&"exec echo aviso >&2; exit 0".to_owned()),
        "{log:?}"
    );
}

#[test]
fn launch_com_comando_que_nao_existe_acaba_com_127_e_guarda_o_stderr() {
    let teste = Teste::novo("launch-127");
    teste.conhecer_servidor();
    teste.modo.exec_imediato.store(true, Ordering::SeqCst);
    let app = teste
        .builder()
        .password("senha")
        .launch("programa-que-nao-existe-m37");
    assert_eq!(esperar_o_fim(&app), RemoteAppStatus::Exited(Some(127)));
    assert!(
        app.stderr_tail().contains("programa-que-nao-existe-m37"),
        "{}",
        app.stderr_tail()
    );
}

#[test]
fn launch_guarda_so_o_fim_do_stderr() {
    let teste = Teste::novo("launch-stderr");
    teste.conhecer_servidor();
    teste.modo.exec_imediato.store(true, Ordering::SeqCst);
    let app = teste
        .builder()
        .password("senha")
        .launch("i=0; while [ $i -lt 600 ]; do echo linha-numero-$i >&2; i=$((i+1)); done");
    assert_eq!(esperar_o_fim(&app), RemoteAppStatus::Exited(Some(0)));
    let fim = app.stderr_tail();
    assert!(fim.len() <= 4096, "{}", fim.len());
    assert!(fim.trim_end().ends_with("linha-numero-599"), "{fim}");
    assert!(!fim.contains("linha-numero-0\n"));
}

#[test]
fn launch_com_senha_errada_falha_com_o_motivo() {
    let teste = Teste::novo("launch-senha");
    teste.conhecer_servidor();
    let app = teste.builder().password("outra").launch("true");
    assert_eq!(
        esperar_o_fim(&app),
        RemoteAppStatus::Failed("autenticação recusada para ana".into())
    );
}

#[test]
fn launch_leva_um_canal_x11_ao_servidor_x() {
    let teste = Teste::novo("launch-x11");
    teste.conhecer_servidor();
    teste.modo.exec_imediato.store(true, Ordering::SeqCst);
    let numero = display_livre();
    let recebido = servidor_x(numero);
    let arquivo = escrever_xauthority(
        &teste,
        xauthority(0, &[127, 0, 0, 1], numero, &cookie_real()),
    );
    let app = teste
        .builder()
        .password("senha")
        .x11(
            X11::trusted()
                .display(format!("127.0.0.1:{numero}"))
                .auth_file(arquivo),
        )
        .launch("sleep 1 # x11");
    assert_eq!(esperar_o_fim(&app), RemoteAppStatus::Exited(Some(0)));
    assert!(
        teste.log().contains(&"x11 ok ok".to_owned()),
        "{:?}",
        teste.log()
    );
    assert!(contem(&recebido.0.lock().unwrap(), &cookie_real()));
}

#[test]
fn launch_sobe_os_tuneis_do_builder() {
    let teste = Teste::novo("launch-tunel");
    teste.conhecer_servidor();
    teste.modo.exec_imediato.store(true, Ordering::SeqCst);
    let alvo = eco();
    let porta = porta_livre();
    let app = teste
        .builder()
        .password("senha")
        .forward(Forward::local(porta, "127.0.0.1", alvo))
        .launch("sleep 3");
    esperar(|| TcpStream::connect(("127.0.0.1", porta)).is_ok());
    ecoar(&format!("127.0.0.1:{porta}"), b"pelo launch");
    app.close();
    esperar_o_fim(&app);
}

#[test]
fn close_encerra_o_launch_que_ainda_roda() {
    let teste = Teste::novo("launch-close");
    teste.conhecer_servidor();
    teste.modo.exec_imediato.store(true, Ordering::SeqCst);
    let app = teste.builder().password("senha").launch("sleep 30");
    esperar(|| app.status() == RemoteAppStatus::Running);
    assert_eq!(app.status(), RemoteAppStatus::Running);
    let antes = Instant::now();
    app.close();
    assert_eq!(esperar_o_fim(&app), RemoteAppStatus::Exited(None));
    assert!(antes.elapsed() < Duration::from_secs(5));
}

#[test]
fn launch_passa_pelo_salto() {
    let (salto, destino) = (Teste::novo("launch-salto"), Teste::novo("launch-destino"));
    for t in [&salto, &destino] {
        t.conhecer_servidor();
    }
    destino.modo.exec_imediato.store(true, Ordering::SeqCst);
    let app = destino
        .builder()
        .password("senha")
        .jump(salto.builder().password("senha"))
        .launch("exit 4");
    assert_eq!(esperar_o_fim(&app), RemoteAppStatus::Exited(Some(4)));
    assert_eq!(salto.log(), [format!("túnel 127.0.0.1:{}", destino.porta)]);
}
