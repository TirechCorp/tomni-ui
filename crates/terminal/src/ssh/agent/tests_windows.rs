//! Os testes do agente que só valem no Windows: o pipe com dono, o pid de quem conecta e o
//! Pageant (pipe e janela), com o crate `pageant` como cliente (o mesmo que o `russh` usa para
//! falar com o Pageant do PuTTY).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::*;
use crate::ssh::keys::{KeyGen, KeyType};

/// Só um Pageant por vez no sistema: os testes que ligam o Pageant esperam a vez.
static PAGEANT: Mutex<()> = Mutex::new(());

fn vez() -> MutexGuard<'static, ()> {
    PAGEANT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Um nome de pipe só deste teste.
fn pipe_do_teste(nome: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    PathBuf::from(format!(
        r"\\.\pipe\tomni-ui-teste-{nome}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ))
}

fn agente_com_uma_chave(nome: &str) -> (Agent, PublicKey) {
    let agente = Agent::start(Some(pipe_do_teste(nome))).expect("subir o agente");
    let chave = KeyGen::new(KeyType::Ed25519)
        .comment("ana@pageant")
        .generate()
        .unwrap();
    agente
        .add(
            chave.private_key().clone(),
            "ana@pageant",
            Constraints::default(),
        )
        .expect("guardar");
    (agente, chave.public_key().clone())
}

/// Liga o Pageant; `None` (e o teste sai sem falhar) se o PuTTY ou outro programa já tem o dele.
fn ligar(agente: &Agent) -> Option<PageantGuard> {
    match agente.start_pageant() {
        Ok(guarda) => Some(guarda),
        Err(PageantError::OutroPageant) => {
            eprintln!("já há um Pageant neste Windows: teste pulado");
            None
        }
        Err(e) => panic!("ligar o Pageant: {e}"),
    }
}

/// `SSH_AGENTC_REQUEST_IDENTITIES` por `fluxo`; devolve o corpo da resposta.
async fn listar<S: AsyncRead + AsyncWrite + Unpin>(fluxo: &mut S) -> Vec<u8> {
    fluxo.write_all(&[0, 0, 0, 1, 11]).await.unwrap();
    fluxo.flush().await.unwrap();
    let mut tamanho = [0u8; 4];
    fluxo.read_exact(&mut tamanho).await.unwrap();
    let mut corpo = vec![0u8; u32::from_be_bytes(tamanho) as usize];
    fluxo.read_exact(&mut corpo).await.unwrap();
    corpo
}

/// A resposta de `listar` tem uma identidade e é a chave `publica`.
fn confere_uma_chave(corpo: &[u8], publica: &PublicKey) {
    let mut r = Reader(corpo);
    assert_eq!(r.u8(), Some(wire::IDENTITIES_ANSWER));
    assert_eq!(r.u32(), Some(1));
    let blob = r.string().expect("o blob da chave");
    assert_eq!(blob, publica.to_bytes().unwrap().as_slice());
    assert_eq!(r.string(), Some(&b"ana@pageant"[..]));
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn pageant_pelo_pipe_lista_a_chave() {
    let _vez = vez();
    let (agente, publica) = agente_com_uma_chave("pageant-pipe");
    let Some(_pageant) = ligar(&agente) else {
        return;
    };
    runtime().block_on(async {
        let mut fluxo = ::pageant::namedpipes::PageantStream::new()
            .await
            .expect("abrir o pipe do Pageant");
        confere_uma_chave(&listar(&mut fluxo).await, &publica);
    });
}

#[test]
fn pageant_pela_janela_lista_a_chave() {
    let _vez = vez();
    let (agente, publica) = agente_com_uma_chave("pageant-janela");
    let Some(_pageant) = ligar(&agente) else {
        return;
    };
    runtime().block_on(async {
        let mut fluxo = ::pageant::wmmessage::PageantStream::new()
            .await
            .expect("falar com a janela do Pageant");
        confere_uma_chave(&listar(&mut fluxo).await, &publica);
    });
}

#[test]
fn pageant_nao_disputa_com_outro_pageant() {
    let _vez = vez();
    let (primeiro, _) = agente_com_uma_chave("pageant-um");
    let (segundo, _) = agente_com_uma_chave("pageant-dois");
    let Some(guarda) = ligar(&primeiro) else {
        return;
    };
    assert!(matches!(
        segundo.start_pageant(),
        Err(PageantError::OutroPageant)
    ));
    // O primeiro continua atendendo.
    runtime().block_on(async {
        let mut fluxo = ::pageant::namedpipes::PageantStream::new().await.unwrap();
        assert_eq!(listar(&mut fluxo).await[0], wire::IDENTITIES_ANSWER);
    });
    // Solto o primeiro, o nome fica livre para o segundo.
    drop(guarda);
    let Some(_guarda) = ligar(&segundo) else {
        return;
    };
}

#[test]
fn pageant_desligado_ao_soltar_a_guarda() {
    let _vez = vez();
    let (agente, _) = agente_com_uma_chave("pageant-solta");
    let Some(guarda) = ligar(&agente) else {
        return;
    };
    assert!(tomni_ui_win32::window_exists("Pageant", "Pageant"));
    drop(guarda);
    assert!(!tomni_ui_win32::window_exists("Pageant", "Pageant"));
}

#[test]
fn pid_no_pipe_do_agente() {
    let (agente, _) = agente_com_uma_chave("pid");
    let pedidos = Arc::new(Mutex::new(Vec::new()));
    let guardados = Arc::clone(&pedidos);
    agente.on_confirm(move |pedido| {
        guardados.lock().unwrap().push(pedido.clone());
        true
    });
    let chave = KeyGen::new(KeyType::Ed25519)
        .comment("ana@pid")
        .generate()
        .unwrap();
    let mut cliente = AgentConnection::connect(agente.path()).expect("conectar ao pipe");
    cliente
        .add(
            chave.private_key(),
            "ana@pid",
            Constraints {
                lifetime: None,
                confirm: true,
            },
        )
        .unwrap();
    cliente.sign(chave.public_key(), b"dados", 0).unwrap();

    let pedidos = pedidos.lock().unwrap();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(pedidos[0].peer.pid, Some(std::process::id()));
    assert!(
        !pedidos[0].peer.chain.is_empty(),
        "a cadeia traz ao menos o programa de teste"
    );
}

#[test]
fn um_segundo_agente_no_mesmo_pipe_e_recusado() {
    let caminho = pipe_do_teste("repetido");
    let _primeiro = Agent::start(Some(caminho.clone())).unwrap();
    let erro = Agent::start(Some(caminho)).unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::AddrInUse);
}
