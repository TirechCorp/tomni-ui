//! Os testes do agente: o protocolo falado à mão e pelo [`AgentConnection`], as restrições, o
//! travamento, o socket (permissões, sobra, dono) e a interoperabilidade com o `ssh-add` e o
//! `ssh-keygen` do sistema, quando existem.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use russh::keys::signature::Verifier;
use ssh_key::encoding::Decode;
use ssh_key::private::KeypairData;
use ssh_key::{Algorithm, Certificate};

use super::*;
use crate::ssh::keys::{KeyGen, KeyType, RsaBits};

/// Uma pasta temporária por teste, apagada no fim.
struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let pasta = std::env::temp_dir().join(format!(
            "tomni-ui-agente-{nome}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).unwrap();
        Self(pasta)
    }

    fn arquivo(&self, nome: &str) -> PathBuf {
        self.0.join(nome)
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn agente(pasta: &Pasta) -> Agent {
    Agent::start(Some(pasta.arquivo("agent.sock"))).unwrap()
}

fn ed25519() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn conectar(agente: &Agent) -> AgentConnection {
    AgentConnection::connect(agente.path()).unwrap()
}

/// Confere a assinatura (`Ok`) de `dados` com a chave pública.
fn confere(public: &PublicKey, dados: &[u8], assinatura: &Signature) -> Result<(), ()> {
    Verifier::verify(public, dados, assinatura).map_err(|_| ())
}

fn assinatura(bytes: &[u8]) -> Signature {
    Signature::decode(&mut &bytes[..]).unwrap()
}

/// Manda uma moldura e devolve a resposta, sem passar pelo cliente.
fn bruto(agente: &Agent, mensagem: &[u8]) -> Vec<u8> {
    let mut socket = UnixStream::connect(agente.path()).unwrap();
    socket.write_all(&wire::framed(mensagem)).unwrap();
    let mut tamanho = [0u8; 4];
    socket.read_exact(&mut tamanho).unwrap();
    let mut resposta = vec![0u8; u32::from_be_bytes(tamanho) as usize];
    socket.read_exact(&mut resposta).unwrap();
    resposta
}

fn esperar(mut condicao: impl FnMut() -> bool) {
    let limite = Instant::now() + Duration::from_secs(5);
    while !condicao() {
        assert!(Instant::now() < limite, "a condição não veio a tempo");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Roda um programa do sistema com o agente em `SSH_AUTH_SOCK`; `None` se ele não existe.
fn rodar(agente: &Agent, programa: &str, args: &[&str]) -> Option<(bool, String, String)> {
    match Command::new(programa)
        .args(args)
        .env("SSH_AUTH_SOCK", agente.path())
        .env_remove("SSH_ASKPASS")
        .output()
    {
        Ok(saida) => Some((
            saida.status.success(),
            String::from_utf8_lossy(&saida.stdout).into_owned(),
            String::from_utf8_lossy(&saida.stderr).into_owned(),
        )),
        Err(_) => {
            eprintln!("{programa} não existe neste sistema: teste pulado");
            None
        }
    }
}

#[test]
fn adiciona_lista_e_remove_pelo_protocolo() {
    let pasta = Pasta::nova("basico");
    let agente = agente(&pasta);
    let mut cliente = conectar(&agente);
    assert!(cliente.list().unwrap().is_empty());

    let (a, b) = (ed25519(), ed25519());
    cliente.add(&a, "primeira", Constraints::default()).unwrap();
    cliente.add(&b, "segunda", Constraints::default()).unwrap();
    let lista = cliente.list().unwrap();
    assert_eq!(lista.len(), 2);
    assert_eq!(lista[0].1, "primeira");
    assert_eq!(lista[1].0, *b.public_key());
    // O `Agent` enxerga o mesmo.
    let doagente = agente.list();
    assert_eq!(doagente.len(), 2);
    assert_eq!(doagente[0].comment, "primeira");
    assert!(doagente[0].expires.is_none() && !doagente[0].confirm);

    // A mesma chave de novo troca a anterior (e o comentário).
    cliente
        .add(&a, "renomeada", Constraints::default())
        .unwrap();
    let lista = cliente.list().unwrap();
    assert_eq!(lista.len(), 2);
    assert!(lista.iter().any(|(_, c)| c == "renomeada"));

    cliente.remove(a.public_key()).unwrap();
    assert_eq!(cliente.list().unwrap().len(), 1);
    assert!(cliente.remove(a.public_key()).is_err(), "já saiu");
    cliente.remove_all().unwrap();
    assert!(cliente.list().unwrap().is_empty());
}

#[test]
fn a_api_do_agente_e_o_socket_veem_as_mesmas_chaves() {
    let pasta = Pasta::nova("api");
    let agente = agente(&pasta);
    let chave = ed25519();
    agente
        .add(chave.clone(), "da-api", Constraints::default())
        .expect("guardar");
    assert_eq!(conectar(&agente).list().unwrap()[0].1, "da-api");
    assert!(agente.remove(chave.public_key()));
    assert!(!agente.remove(chave.public_key()));
    agente
        .add(chave, "de-novo", Constraints::default())
        .expect("guardar");
    agente.remove_all();
    assert!(agente.list().is_empty());
}

#[test]
fn assina_ed25519_e_a_assinatura_confere() {
    let pasta = Pasta::nova("assina");
    let agente = agente(&pasta);
    let chave = ed25519();
    let mut cliente = conectar(&agente);
    cliente.add(&chave, "x", Constraints::default()).unwrap();
    let bytes = cliente.sign(chave.public_key(), b"desafio", 0).unwrap();
    let assinatura = assinatura(&bytes);
    assert_eq!(assinatura.algorithm(), Algorithm::Ed25519);
    confere(chave.public_key(), b"desafio", &assinatura).unwrap();
    assert!(confere(chave.public_key(), b"outro", &assinatura).is_err());
    // Uma chave que o agente não tem não assina.
    assert!(cliente.sign(ed25519().public_key(), b"x", 0).is_err());
}

#[test]
fn rsa_assina_com_o_hash_pedido_nas_flags() {
    let pasta = Pasta::nova("rsa");
    let agente = agente(&pasta);
    let chave = KeyGen::new(KeyType::Rsa(RsaBits::B3072))
        .generate()
        .unwrap()
        .private_key()
        .clone();
    let mut cliente = conectar(&agente);
    cliente.add(&chave, "rsa", Constraints::default()).unwrap();
    for (flags, hash) in [
        (2, Some(HashAlg::Sha256)),
        (4, Some(HashAlg::Sha512)),
        (0, None),
    ] {
        let bytes = cliente.sign(chave.public_key(), b"dados", flags).unwrap();
        let assinatura = assinatura(&bytes);
        assert_eq!(assinatura.algorithm(), Algorithm::Rsa { hash }, "{flags}");
        confere(chave.public_key(), b"dados", &assinatura).unwrap();
    }
}

#[test]
fn travado_lista_vazio_e_nao_assina_nem_aceita() {
    let pasta = Pasta::nova("trava");
    let agente = agente(&pasta);
    let chave = ed25519();
    let mut cliente = conectar(&agente);
    cliente.add(&chave, "x", Constraints::default()).unwrap();

    cliente.lock("segredo").unwrap();
    assert!(agente.is_locked());
    assert!(cliente.lock("outra").is_err(), "já está travado");
    // Vazia, não `FAILURE`: o `ssh` entende como "sem chaves".
    assert!(cliente.list().unwrap().is_empty());
    assert!(cliente.sign(chave.public_key(), b"x", 0).is_err());
    assert!(
        cliente
            .add(&ed25519(), "y", Constraints::default())
            .is_err()
    );
    assert!(cliente.remove(chave.public_key()).is_err());
    assert!(cliente.remove_all().is_err());

    assert!(cliente.unlock("errada").is_err());
    assert!(!agente.unlock("errada"));
    assert!(agente.is_locked());
    cliente.unlock("segredo").unwrap();
    assert!(!agente.is_locked());
    assert_eq!(cliente.list().unwrap().len(), 1);
    assert!(cliente.unlock("segredo").is_err(), "já está destravado");
    // A API faz o mesmo.
    assert!(agente.lock("api"));
    assert!(!agente.lock("api"));
    assert!(agente.unlock("api"));
}

#[test]
fn a_chave_com_validade_some_sozinha_e_avisa() {
    let pasta = Pasta::nova("validade");
    let agente = agente(&pasta);
    let avisos = Arc::new(AtomicUsize::new(0));
    let contador = Arc::clone(&avisos);
    agente.on_change(move || {
        contador.fetch_add(1, Ordering::SeqCst);
    });
    let chave = ed25519();
    let mut cliente = conectar(&agente);
    let restricoes = Constraints {
        lifetime: Some(Duration::from_secs(1)),
        confirm: false,
    };
    cliente.add(&chave, "vida-curta", restricoes).unwrap();
    assert_eq!(avisos.load(Ordering::SeqCst), 1, "a adição avisa");
    let chaves = agente.list();
    assert_eq!(chaves.len(), 1);
    assert!(chaves[0].expires.is_some());
    esperar(|| agente.list().is_empty());
    assert!(cliente.list().unwrap().is_empty());
    assert!(avisos.load(Ordering::SeqCst) >= 2, "a expiração avisa");
}

#[test]
fn a_validade_padrao_vale_so_para_a_chave_sem_validade_propria() {
    let pasta = Pasta::nova("validade-padrao");
    let agente = agente(&pasta);
    agente.default_lifetime(Some(Duration::from_secs(600)));
    let mut cliente = conectar(&agente);
    cliente
        .add(&ed25519(), "padrao", Constraints::default())
        .unwrap();
    let propria = Constraints {
        lifetime: Some(Duration::from_secs(60)),
        confirm: false,
    };
    cliente.add(&ed25519(), "propria", propria).unwrap();
    let chaves = agente.list();
    let vida = |comentario: &str| {
        let chave = chaves.iter().find(|c| c.comment == comentario).unwrap();
        chave.expires.unwrap().duration_since(chave.added).unwrap()
    };
    assert_eq!(vida("padrao"), Duration::from_secs(600));
    assert_eq!(vida("propria"), Duration::from_secs(60));
}

#[test]
fn a_validade_nao_apaga_uma_chave_adicionada_de_novo() {
    let pasta = Pasta::nova("validade-nova");
    let agente = agente(&pasta);
    let chave = ed25519();
    let curta = Constraints {
        lifetime: Some(Duration::from_millis(250)),
        confirm: false,
    };
    agente.add(chave.clone(), "curta", curta).expect("guardar");
    // A mesma chave, sem validade, antes de a primeira vencer.
    agente
        .add(chave, "longa", Constraints::default())
        .expect("guardar");
    std::thread::sleep(Duration::from_millis(600));
    let chaves = agente.list();
    assert_eq!(chaves.len(), 1);
    assert_eq!(chaves[0].comment, "longa");
}

#[test]
fn confirmacao_so_assina_se_quem_responde_permite() {
    let pasta = Pasta::nova("confirma");
    let agente = agente(&pasta);
    let chave = ed25519();
    let mut cliente = conectar(&agente);
    let pedir = Constraints {
        lifetime: None,
        confirm: true,
    };
    cliente.add(&chave, "pede", pedir).unwrap();
    assert!(agente.list()[0].confirm);

    // Sem quem responda, nega.
    assert!(cliente.sign(chave.public_key(), b"1", 0).is_err());

    let perguntas = Arc::new(AtomicUsize::new(0));
    let permitir = Arc::new(AtomicBool::new(false));
    let (contador, resposta) = (Arc::clone(&perguntas), Arc::clone(&permitir));
    agente.on_confirm(move |pedido| {
        assert_eq!(pedido.key.comment, "pede");
        contador.fetch_add(1, Ordering::SeqCst);
        resposta.load(Ordering::SeqCst)
    });
    assert!(cliente.sign(chave.public_key(), b"2", 0).is_err());
    permitir.store(true, Ordering::SeqCst);
    let bytes = cliente.sign(chave.public_key(), b"3", 0).unwrap();
    confere(chave.public_key(), b"3", &assinatura(&bytes)).unwrap();
    assert_eq!(perguntas.load(Ordering::SeqCst), 2);
}

#[test]
fn a_confirmacao_pendente_nao_trava_os_outros_clientes() {
    let pasta = Pasta::nova("confirma-pendente");
    let agente = agente(&pasta);
    let (livre, presa) = (ed25519(), ed25519());
    let mut cliente = conectar(&agente);
    cliente
        .add(&livre, "livre", Constraints::default())
        .unwrap();
    let pedir = Constraints {
        lifetime: None,
        confirm: true,
    };
    cliente.add(&presa, "presa", pedir).unwrap();
    let (liberar, espera) = std::sync::mpsc::channel::<()>();
    let espera = Mutex::new(espera);
    agente.on_confirm(move |_| {
        let _ = espera.lock().unwrap().recv_timeout(Duration::from_secs(10));
        true
    });
    let publica = presa.public_key().clone();
    let agente2 = agente.clone();
    let esperando = std::thread::spawn(move || conectar(&agente2).sign(&publica, b"x", 0).is_ok());
    std::thread::sleep(Duration::from_millis(200));
    // Enquanto a pergunta espera, o agente atende outro.
    let inicio = Instant::now();
    assert!(cliente.sign(livre.public_key(), b"y", 0).is_ok());
    assert!(inicio.elapsed() < Duration::from_secs(2));
    liberar.send(()).unwrap();
    assert!(esperando.join().unwrap());
}

#[test]
fn extensoes_e_mensagens_desconhecidas() {
    let pasta = Pasta::nova("extensoes");
    let agente = agente(&pasta);
    let extensao = |nome: &str| {
        let mut w = Writer::new(wire::EXTENSION);
        w.string(nome.as_bytes());
        bruto(&agente, &w.0)
    };
    assert_eq!(extensao("query"), [wire::SUCCESS]);
    // O `session-bind` sem corpo não se entende; com o corpo inteiro, é aceito.
    assert_eq!(extensao("session-bind@openssh.com"), [wire::FAILURE]);
    let servidor = ed25519();
    let mut bind = Writer::new(wire::EXTENSION);
    bind.string(b"session-bind@openssh.com")
        .string(&servidor.public_key().to_bytes().unwrap())
        .string(b"id-da-sessao")
        .string(b"assinatura")
        .u8(0);
    assert_eq!(bruto(&agente, &bind.0), [wire::SUCCESS]);
    assert_eq!(extensao("inventada@exemplo"), [wire::FAILURE]);
    assert_eq!(bruto(&agente, &[99]), [wire::FAILURE]);
    // Pedido sem corpo.
    assert_eq!(bruto(&agente, &[wire::SIGN_REQUEST]), [wire::FAILURE]);
    assert_eq!(bruto(&agente, &[wire::ADD_IDENTITY]), [wire::FAILURE]);
}

#[test]
fn restricao_desconhecida_da_failure() {
    let pasta = Pasta::nova("restricao");
    let agente = agente(&pasta);
    let chave = ed25519();
    let mut pedido = Writer::new(wire::ADD_ID_CONSTRAINED);
    {
        use ssh_key::encoding::Encode;
        chave.key_data().encode(&mut pedido.0).unwrap();
    }
    pedido.string(b"c").u8(255).string(b"foo@bar").string(b"");
    assert_eq!(bruto(&agente, &pedido.0), [wire::FAILURE]);
    assert!(agente.list().is_empty());
}

/// Um certificado de usuário da `chave`, assinado por uma CA nova.
fn certificar(chave: &PrivateKey) -> Certificate {
    let agora = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut construtor = ssh_key::certificate::Builder::new_with_random_nonce(
        &mut rand::rng(),
        chave.public_key().clone(),
        agora - 60,
        agora + 3600,
    )
    .unwrap();
    construtor.serial(7).unwrap();
    construtor.key_id("ana").unwrap();
    construtor.valid_principal("ana").unwrap();
    construtor.sign(&ed25519()).unwrap()
}

/// O corpo do `ADD_IDENTITY` de um certificado, como o `ssh-add` manda: o certificado, os campos
/// privados da chave (sem o nome do algoritmo) e o comentário.
fn pedido_de_certificado(chave: &PrivateKey, cert: &Certificate) -> Vec<u8> {
    let mut pedido = Writer::new(wire::ADD_IDENTITY);
    pedido
        .string(cert.algorithm().to_certificate_type().as_bytes())
        .string(&cert.to_bytes().unwrap());
    let KeypairData::Ed25519(par) = chave.key_data() else {
        unreachable!("o teste usa Ed25519");
    };
    pedido
        .string(par.public.as_ref())
        .string(&[&par.private.to_bytes()[..], par.public.as_ref()].concat())
        .string(b"cert");
    pedido.0
}

#[test]
fn certificado_so_entra_com_a_chave_presente() {
    let pasta = Pasta::nova("certificado");
    let agente = agente(&pasta);
    let chave = ed25519();
    let cert = certificar(&chave);
    let pedido = pedido_de_certificado(&chave, &cert);

    // Sem a privada no agente, o certificado não tem como assinar: recusa.
    assert_eq!(bruto(&agente, &pedido), [wire::FAILURE]);
    assert!(agente.list().is_empty());

    agente
        .add(chave.clone(), "chave", Constraints::default())
        .expect("guardar");
    assert_eq!(bruto(&agente, &pedido), [wire::SUCCESS]);
    let lista = agente.list();
    assert_eq!(lista.len(), 2, "a chave e o certificado");
    assert!(lista[0].certificate.is_none());
    assert_eq!(lista[1].certificate.as_ref(), Some(&cert));
    assert_eq!(lista[1].comment, "cert");

    // O certificado assina com a chave dele.
    let mut cliente = conectar(&agente);
    let identidades = cliente.list().unwrap();
    assert_eq!(identidades.len(), 2);

    // Tirar o certificado deixa a chave; tirar a chave leva os certificados.
    assert!(agente.shared.remove_blob(&cert.to_bytes().unwrap()));
    assert_eq!(agente.list().len(), 1);
    assert_eq!(bruto(&agente, &pedido), [wire::SUCCESS]);
    assert!(agente.remove(chave.public_key()));
    assert!(agente.list().is_empty());
}

#[test]
fn adicionar_a_chave_de_novo_troca_tambem_os_certificados() {
    let pasta = Pasta::nova("certificado-troca");
    let agente = agente(&pasta);
    let chave = ed25519();
    let cert = certificar(&chave);
    agente
        .add(chave.clone(), "um", Constraints::default())
        .expect("guardar");
    assert!(agente.add_certificate(cert, "cert"));
    assert_eq!(agente.list().len(), 2);
    agente
        .add(chave, "dois", Constraints::default())
        .expect("guardar");
    let lista = agente.list();
    assert_eq!(lista.len(), 1, "o certificado velho saiu com a chave velha");
    assert_eq!(lista[0].comment, "dois");
}

#[test]
fn chave_de_seguranca_e_recusada() {
    let pasta = Pasta::nova("sk");
    let agente = agente(&pasta);
    let chave = ed25519();
    let KeypairData::Ed25519(par) = chave.key_data() else {
        unreachable!();
    };
    // Os campos de uma `sk-ssh-ed25519@openssh.com`: pública, aplicação, flags, handle e reservado.
    let mut pedido = Writer::new(wire::ADD_IDENTITY);
    pedido
        .string(b"sk-ssh-ed25519@openssh.com")
        .string(par.public.as_ref())
        .string(b"ssh:")
        .u8(1)
        .string(b"handle")
        .string(b"")
        .string(b"sk");
    assert_eq!(bruto(&agente, &pedido.0), [wire::FAILURE]);
    assert!(agente.list().is_empty());
}

#[test]
fn os_erros_do_agente_falam_em_portugues() {
    assert!(AgentError::SecurityKey.to_string().contains("token FIDO"));
    assert_eq!(
        AgentError::Invalid("curta".to_owned()).to_string(),
        "A chave não pôde ser guardada: curta"
    );
}

#[test]
fn uma_moldura_grande_demais_fecha_so_aquela_conexao() {
    let pasta = Pasta::nova("moldura");
    let agente = agente(&pasta);
    let mut socket = UnixStream::connect(agente.path()).unwrap();
    socket
        .write_all(&((wire::MAX_FRAME as u32) + 1).to_be_bytes())
        .unwrap();
    let mut resto = Vec::new();
    // O agente fecha: a leitura termina sem resposta.
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    assert_eq!(socket.read_to_end(&mut resto).unwrap_or(0), 0);
    assert!(resto.is_empty());
    // E segue atendendo.
    assert!(conectar(&agente).list().unwrap().is_empty());
}

#[test]
fn o_socket_e_a_pasta_tem_as_permissoes_do_openssh() {
    let pasta = Pasta::nova("permissoes");
    let caminho = pasta.arquivo("novo/agent.sock");
    let agente = Agent::start(Some(caminho.clone())).unwrap();
    let modo = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(modo(&caminho), 0o600);
    assert_eq!(modo(caminho.parent().unwrap()), 0o700);
    assert_eq!(agente.path(), caminho);
}

#[cfg(unix)]
#[test]
fn um_caminho_longo_da_erro_em_portugues() {
    let pasta = Pasta::nova("agente-longo");
    let longo = pasta.arquivo(&"p".repeat(200)).join("agent.sock");
    let erro = Agent::start(Some(longo.clone())).expect_err("deveria falhar");
    assert_eq!(erro.kind(), io::ErrorKind::InvalidInput);
    let texto = erro.to_string();
    assert!(texto.contains("O caminho do socket tem"), "{texto}");
    assert!(texto.contains("bytes, e o sistema aceita até"), "{texto}");
    // Nada foi criado pelo caminho.
    assert!(!longo.parent().unwrap().exists());
}

#[test]
fn parar_apaga_o_socket_e_o_ultimo_clone_tambem() {
    let pasta = Pasta::nova("parar");
    let caminho = pasta.arquivo("agent.sock");
    let agente = Agent::start(Some(caminho.clone())).unwrap();
    assert!(caminho.exists());
    agente.stop();
    assert!(!caminho.exists());
    assert!(AgentConnection::connect(&caminho).is_err());

    let outro = Agent::start(Some(caminho.clone())).unwrap();
    let clone = outro.clone();
    drop(outro);
    assert!(caminho.exists(), "um clone ainda segura o agente");
    drop(clone);
    assert!(!caminho.exists());
}

#[test]
fn socket_que_sobrou_e_refeito_e_agente_vivo_nao_e_tomado() {
    let pasta = Pasta::nova("sobra");
    let caminho = pasta.arquivo("agent.sock");
    // Sobra: um socket sem ninguém atrás. Outro teste que cria um processo filho (`sh`,
    // `ssh-keygen`) no mesmo instante herda por um momento o descritor do `UnixListener`, e aí
    // alguém ainda "responde" no socket: se for esse o caso, refaz a sobra.
    let mut tentativas = 0;
    let agente = loop {
        drop(std::os::unix::net::UnixListener::bind(&caminho).unwrap());
        assert!(caminho.exists());
        match Agent::start(Some(caminho.clone())) {
            Ok(agente) => break agente,
            Err(erro) if erro.kind() == io::ErrorKind::AddrInUse && tentativas < 20 => {
                tentativas += 1;
                std::thread::sleep(Duration::from_millis(25));
                std::fs::remove_file(&caminho).unwrap();
            }
            Err(erro) => panic!("o agente não subiu sobre a sobra: {erro}"),
        }
    };
    assert!(conectar(&agente).list().unwrap().is_empty());

    let erro = Agent::start(Some(caminho.clone())).unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::AddrInUse);
    assert_eq!(
        erro.to_string(),
        format!("Já há um agente em {}", caminho.display())
    );
    // O primeiro segue de pé.
    assert!(conectar(&agente).list().unwrap().is_empty());
}

#[test]
fn o_caminho_padrao_usa_o_pid_e_a_pasta_do_agente() {
    let agente = Agent::start(None).unwrap();
    let caminho = agente.path().to_owned();
    assert_eq!(
        caminho.file_name().unwrap().to_string_lossy(),
        format!("agent.{}", std::process::id())
    );
    assert!(caminho.to_string_lossy().contains("tomni-ui"));
    let pasta = caminho.parent().unwrap();
    assert_eq!(
        std::fs::metadata(pasta).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(agente);
    assert!(!caminho.exists());
}

#[test]
fn o_ssh_add_do_sistema_usa_o_nosso_agente() {
    let pasta = Pasta::nova("ssh-add");
    let agente = agente(&pasta);
    let chave = KeyGen::new(KeyType::Ed25519)
        .comment("ana@sshadd")
        .generate()
        .unwrap();
    let arquivo = pasta.arquivo("id");
    chave.write(&arquivo, false).unwrap();
    let arquivo = arquivo.to_str().unwrap();

    let Some((ok, _, erro)) = rodar(&agente, "ssh-add", &[arquivo]) else {
        return;
    };
    assert!(ok, "ssh-add: {erro}");
    let lista = agente.list();
    assert_eq!(lista.len(), 1);
    assert_eq!(lista[0].comment, "ana@sshadd");
    assert_eq!(lista[0].public, *chave.public_key());

    let (ok, saida, _) = rodar(&agente, "ssh-add", &["-l"]).unwrap();
    assert!(ok);
    assert!(saida.contains(&chave.fingerprint()), "{saida}");
    let (_, saida, _) = rodar(&agente, "ssh-add", &["-L"]).unwrap();
    assert_eq!(saida.trim(), chave.public_openssh());

    // Validade e confirmação.
    agente.remove_all();
    let (ok, _, erro) = rodar(&agente, "ssh-add", &["-t", "3600", "-c", arquivo]).unwrap();
    assert!(ok, "{erro}");
    let guardada = &agente.list()[0];
    assert!(guardada.confirm);
    assert!(guardada.expires.is_some());

    // Travar e destravar pelo `ssh-add -x` pede a senha no terminal; a API cobre. Remover:
    let (ok, _, _) = rodar(&agente, "ssh-add", &["-d", arquivo]).unwrap();
    assert!(ok);
    assert!(agente.list().is_empty());
    let (ok, _, _) = rodar(&agente, "ssh-add", &[arquivo]).unwrap();
    assert!(ok);
    let (ok, _, _) = rodar(&agente, "ssh-add", &["-D"]).unwrap();
    assert!(ok);
    assert!(agente.list().is_empty());
    // Sem chaves, o `ssh-add -l` diz que o agente está vazio (saída 1), não que falhou (2).
    let (ok, _, _) = rodar(&agente, "ssh-add", &["-l"]).unwrap();
    assert!(!ok);
}

#[test]
fn o_ssh_keygen_assina_pelo_nosso_agente_e_confere() {
    let pasta = Pasta::nova("keygen-sign");
    let agente = agente(&pasta);
    for tipo in [KeyType::Ed25519, KeyType::Rsa(RsaBits::B3072)] {
        agente.remove_all();
        let chave = KeyGen::new(tipo).comment("ana@sign").generate().unwrap();
        agente
            .add(
                chave.private_key().clone(),
                "ana@sign",
                Constraints::default(),
            )
            .expect("guardar");
        let publica = pasta.arquivo(&format!("{}.pub", tipo.file_name()));
        std::fs::write(&publica, format!("{}\n", chave.public_openssh())).unwrap();
        let dados = pasta.arquivo(&format!("dados-{}", tipo.file_name()));
        std::fs::write(&dados, "conteúdo").unwrap();
        let Some((ok, _, erro)) = rodar(
            &agente,
            "ssh-keygen",
            &[
                "-Y",
                "sign",
                "-U",
                "-f",
                publica.to_str().unwrap(),
                "-n",
                "teste",
                dados.to_str().unwrap(),
            ],
        ) else {
            return;
        };
        assert!(ok, "{}: {erro}", tipo.label());
        let assinada = format!("{}.sig", dados.display());
        // A conferência lê os dados do stdin.
        let conferencia = Command::new("ssh-keygen")
            .args(["-Y", "check-novalidate", "-n", "teste", "-s", &assinada])
            .stdin(std::fs::File::open(&dados).unwrap())
            .output()
            .unwrap();
        assert!(
            conferencia.status.success(),
            "{}: {}",
            tipo.label(),
            String::from_utf8_lossy(&conferencia.stderr)
        );
        assert!(std::path::Path::new(&assinada).exists(), "{}", tipo.label());
    }
}

#[test]
fn o_cliente_conversa_com_o_ssh_agent_do_sistema() {
    // O contrário: o nosso `AgentConnection` conversa com o `ssh-agent` de verdade.
    let pasta = Pasta::nova("ssh-agent");
    let socket = pasta.arquivo("sistema.sock");
    let Ok(mut filho) = Command::new("ssh-agent")
        .args(["-D", "-a", socket.to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .spawn()
    else {
        eprintln!("ssh-agent não existe neste sistema: teste pulado");
        return;
    };
    esperar(|| socket.exists());
    let chave = ed25519();
    let resultado = (|| -> io::Result<()> {
        let mut cliente = AgentConnection::connect(&socket)?;
        cliente.add(&chave, "no-openssh", Constraints::default())?;
        let lista = cliente.list()?;
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].1, "no-openssh");
        let bytes = cliente.sign(chave.public_key(), b"dados", 0)?;
        confere(chave.public_key(), b"dados", &assinatura(&bytes)).unwrap();
        cliente.lock("segredo")?;
        assert!(cliente.list()?.is_empty());
        cliente.unlock("segredo")?;
        cliente.remove(chave.public_key())?;
        assert!(cliente.list()?.is_empty());
        Ok(())
    })();
    let _ = filho.kill();
    let _ = filho.wait();
    resultado.unwrap();
}

#[test]
fn parse_proc_stat_le_o_pai_e_o_nome_com_espacos_e_parenteses() {
    assert_eq!(
        parse_proc_stat("1234 (ssh-add) S 99 1234 1234 0 -1 4194560"),
        Some((99, "ssh-add".to_owned()))
    );
    assert_eq!(
        parse_proc_stat("7 (um (nome) estranho) R 42 7 7 0 -1"),
        Some((42, "um (nome) estranho".to_owned()))
    );
    assert_eq!(parse_proc_stat("sem parênteses"), None);
    assert_eq!(parse_proc_stat("1 (x)"), None);
}

#[test]
fn script_name_troca_o_interpretador_pelo_script() {
    assert_eq!(
        script_name("node", b"node\0--inspect\0/usr/lib/codex/bin/codex\0run\0"),
        Some("codex".to_owned())
    );
    assert_eq!(
        script_name("python3.12", b"python3\0-u\0/home/ana/tool.py\0"),
        Some("tool.py".to_owned())
    );
    assert_eq!(script_name("node", b"node\0"), None, "sem script");
    assert_eq!(
        script_name("ssh", b"ssh\0host\0"),
        None,
        "não é interpretador"
    );
}

#[test]
fn a_cadeia_sobe_ate_o_init_e_nao_passa_de_seis() {
    let tabela = |pid: u32| match pid {
        50 => Some((40, "ssh".to_owned())),
        40 => Some((30, "git".to_owned())),
        30 => Some((20, "codex".to_owned())),
        20 => Some((10, "systemd".to_owned())),
        _ => None,
    };
    assert_eq!(chain_with(50, tabela), ["ssh", "git", "codex"]);
    assert!(chain_with(1, tabela).is_empty());
    assert!(chain_with(999, tabela).is_empty(), "processo que sumiu");

    // Uma árvore sem fim: corta em seis.
    let funda = |pid: u32| Some((pid + 1, format!("p{pid}")));
    assert_eq!(chain_with(100, funda).len(), 6);
    // Um laço (pai de si mesmo) não trava.
    assert_eq!(chain_with(5, |_| Some((5, "x".to_owned()))), ["x"]);
}

#[cfg(target_os = "linux")]
#[test]
fn process_chain_do_proprio_teste_comeca_pelo_executavel_dele() {
    let cadeia = process_chain(std::process::id());
    assert!(!cadeia.is_empty());
    let eu = std::fs::read_to_string("/proc/self/comm").unwrap();
    assert_eq!(cadeia[0], eu.trim());
}

/// Pede ao `ssh-add -T` (que assina um desafio com a chave) com o agente perguntando; devolve o
/// pedido que chegou na pergunta e o pid do filho.
#[test]
fn pedido_traz_o_pid_e_a_cadeia_do_ssh_add() {
    let pasta = Pasta::nova("peer");
    let agente = agente(&pasta);
    let chave = KeyGen::new(KeyType::Ed25519)
        .comment("ana@peer")
        .generate()
        .unwrap();
    let arquivo = pasta.arquivo("ed");
    chave.write(&arquivo, false).unwrap();
    std::fs::write(pasta.arquivo("ed.pub"), chave.public_openssh()).unwrap();
    agente
        .add(
            chave.private_key().clone(),
            "ana@peer",
            Constraints {
                lifetime: None,
                confirm: true,
            },
        )
        .expect("guardar");

    let pedidos = Arc::new(Mutex::new(Vec::new()));
    let guardados = Arc::clone(&pedidos);
    agente.on_confirm(move |pedido| {
        guardados.lock().unwrap().push(pedido.clone());
        true
    });

    let pub_arquivo = pasta.arquivo("ed.pub");
    let filho = match Command::new("ssh-add")
        .args(["-T", pub_arquivo.to_str().unwrap()])
        .env("SSH_AUTH_SOCK", agente.path())
        .env_remove("SSH_ASKPASS")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(filho) => filho,
        Err(_) => {
            eprintln!("ssh-add não existe neste sistema: teste pulado");
            return;
        }
    };
    let pid = filho.id();
    let saida = filho.wait_with_output().unwrap();
    assert!(
        saida.status.success(),
        "ssh-add -T: {}",
        String::from_utf8_lossy(&saida.stderr)
    );

    let pedidos = pedidos.lock().unwrap();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(pedidos[0].key.comment, "ana@peer");
    assert_eq!(pedidos[0].peer.pid, Some(pid));
    #[cfg(target_os = "linux")]
    assert_eq!(pedidos[0].peer.chain[0], "ssh-add");
    assert!(pedidos[0].peer.host_key.is_none(), "sem session-bind");
}

#[test]
fn ssh_add_l_lista_a_chave() {
    let pasta = Pasta::nova("ssh-add-l");
    let agente = agente(&pasta);
    let chave = ed25519();
    agente
        .add(chave.clone(), "lista", Constraints::default())
        .expect("guardar");
    let Some((ok, saida, erro)) = rodar(&agente, "ssh-add", &["-l"]) else {
        return;
    };
    assert!(ok, "ssh-add -l: {erro}");
    assert!(saida.contains("ED25519"), "{saida}");
    assert!(saida.contains("lista"), "{saida}");
}

#[test]
fn ssh_add_com_certificado_lista_os_dois() {
    let pasta = Pasta::nova("ssh-add-cert");
    let agente = agente(&pasta);
    let dir = pasta.0.to_str().unwrap();
    let gerar = |args: &[&str]| {
        Command::new("ssh-keygen")
            .args(args)
            .current_dir(dir)
            .output()
            .ok()
    };
    // A CA, a chave do usuário (RSA, como o plano pede) e o certificado `rsa-cert.pub`.
    let Some(saida) = gerar(&["-q", "-t", "ed25519", "-N", "", "-f", "ca"]) else {
        eprintln!("ssh-keygen não existe neste sistema: teste pulado");
        return;
    };
    assert!(saida.status.success());
    assert!(
        gerar(&["-q", "-t", "rsa", "-b", "2048", "-N", "", "-f", "rsa"])
            .unwrap()
            .status
            .success()
    );
    let saida = gerar(&["-q", "-s", "ca", "-I", "ana", "-n", "ana", "rsa.pub"]).unwrap();
    assert!(
        saida.status.success(),
        "{}",
        String::from_utf8_lossy(&saida.stderr)
    );

    let Some((ok, _, erro)) = rodar(
        &agente,
        "ssh-add",
        &[pasta.arquivo("rsa").to_str().unwrap()],
    ) else {
        return;
    };
    assert!(ok, "ssh-add: {erro}");
    let (_, saida, _) = rodar(&agente, "ssh-add", &["-l"]).unwrap();
    assert_eq!(saida.lines().count(), 2, "a chave e o certificado: {saida}");
    let lista = agente.list();
    assert_eq!(lista.len(), 2);
    assert!(lista.iter().any(|k| k.certificate.is_some()));
}
