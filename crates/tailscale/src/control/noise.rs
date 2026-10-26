//! O canal Noise do ts2021: o handshake `Noise_IK_25519_ChaChaPoly_BLAKE2s`, o enquadramento
//! em registros e a carga antecipada do servidor.
//!
//! O que o protocolo fixa (e os testes travam com vetores):
//!
//! - **Prólogo:** `"Tailscale Control Protocol v{N}"`, com `N` a versão de capacidade do
//!   cliente em decimal (não é um "1" fixo).
//! - **Pré-mensagem:** a chave pública Noise do servidor (a de `/key`), que o cliente já tem.
//! - **Iniciação (101 bytes):** `[N: u16 BE][tipo 1][tamanho 96: u16 BE]` e depois a efêmera
//!   (32), a estática cifrada (48) e a etiqueta do corpo vazio (16). Vai em base64 no
//!   cabeçalho `X-Tailscale-Handshake` do pedido de upgrade.
//! - **Resposta (51 bytes):** `[tipo 2][tamanho 48: u16 BE]`, a efêmera (32) e a etiqueta (16).
//!   Um erro do servidor vem como `[tipo 3][tamanho: u16 BE][texto]`, sem cifra.
//! - **Registros:** `[tipo 4][tamanho: u16 BE][texto cifrado]`, no máximo 4096 bytes com o
//!   cabeçalho (4077 de texto claro). O nonce é de 12 bytes: 4 zeros e o contador de 64 bits
//!   **big-endian**, que sobe a cada registro (o do Noise padrão seria little-endian). O
//!   cliente cifra com a primeira chave da divisão e decifra com a segunda; o servidor, o
//!   contrário.
//! - **Carga antecipada:** nos primeiros 9 bytes do servidor, ou há o início de um quadro
//!   HTTP/2 (a conversa normal) ou `"\xff\xff\xffTS"`, 4 bytes de tamanho e um JSON
//!   [`EarlyNoise`].

use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use snow::{Builder, HandshakeState};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use zeroize::Zeroizing;

use crate::key::{MachineKey, MachinePublic};
use crate::tailcfg::EarlyNoise;

/// O padrão Noise do ts2021.
const PADRAO: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// O tamanho da mensagem de iniciação, com o cabeçalho.
pub const TAMANHO_INICIACAO: usize = 101;
/// O tamanho da mensagem de resposta, com o cabeçalho.
pub const TAMANHO_RESPOSTA: usize = 51;
/// O corpo da iniciação (sem o cabeçalho de 5 bytes).
const CORPO_INICIACAO: usize = 96;
/// O corpo da resposta (sem o cabeçalho de 3 bytes).
const CORPO_RESPOSTA: usize = 48;
/// O tipo da iniciação.
const TIPO_INICIACAO: u8 = 1;
/// O tipo da resposta.
const TIPO_RESPOSTA: u8 = 2;
/// O tipo da mensagem de erro do servidor.
const TIPO_ERRO: u8 = 3;
/// O tipo de um registro.
const TIPO_REGISTRO: u8 = 4;
/// O tamanho do cabeçalho de um registro.
const CABECALHO: usize = 3;
/// O maior registro, com o cabeçalho.
const MAIOR_REGISTRO: usize = 4096;
/// O maior texto cifrado de um registro.
const MAIOR_CIFRADO: usize = MAIOR_REGISTRO - CABECALHO;
/// A etiqueta do Poly1305.
const ETIQUETA: usize = 16;
/// O maior texto claro de um registro.
pub const MAIOR_CLARO: usize = MAIOR_CIFRADO - ETIQUETA;
/// A marca da carga antecipada.
const MARCA_ANTECIPADA: &[u8; 5] = b"\xff\xff\xffTS";
/// O começo da conversa: a marca e o tamanho, ou o cabeçalho de um quadro HTTP/2.
const INICIO: usize = 9;
/// O maior JSON de carga antecipada aceito (o de verdade tem uns 80 bytes).
const MAIOR_ANTECIPADA: usize = 64 * 1024;

/// O que pode dar errado no canal Noise.
#[derive(Debug)]
pub enum NoiseError {
    /// O handshake falhou (chave errada, mensagem corrompida).
    Handshake(String),
    /// O servidor mandou uma mensagem de erro no lugar da resposta.
    Server(String),
    /// O servidor não segue o formato esperado.
    Protocol(String),
    /// A conexão caiu.
    Io(io::Error),
}

impl fmt::Display for NoiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoiseError::Handshake(m) => {
                write!(f, "o handshake com o servidor de controle falhou: {m}")
            }
            NoiseError::Server(m) => write!(f, "o servidor de controle recusou a conexão: {m}"),
            NoiseError::Protocol(m) => write!(f, "o servidor de controle fugiu do protocolo: {m}"),
            NoiseError::Io(e) => write!(f, "a conexão com o servidor de controle caiu: {e}"),
        }
    }
}

impl std::error::Error for NoiseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NoiseError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for NoiseError {
    fn from(e: io::Error) -> Self {
        NoiseError::Io(e)
    }
}

fn erro_snow(e: snow::Error) -> NoiseError {
    NoiseError::Handshake(e.to_string())
}

/// Um sentido do canal: a chave e o contador do nonce.
pub struct Cipher {
    cifra: ChaCha20Poly1305,
    contador: u64,
}

impl Cipher {
    /// Monta o sentido com a chave que a divisão do handshake deu.
    pub fn new(chave: &[u8; 32]) -> Self {
        Cipher {
            cifra: ChaCha20Poly1305::new(&Key::from(*chave)),
            contador: 0,
        }
    }

    /// O nonce do próximo registro: 4 zeros e o contador em big-endian.
    fn nonce(&self) -> Result<Nonce, io::Error> {
        // O valor máximo fica inválido, como no protocolo: depois dele não há mais nonce.
        if self.contador == u64::MAX {
            return Err(io::Error::other(
                "a chave do canal esgotou os nonces; reconecte",
            ));
        }
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.contador.to_be_bytes());
        Ok(Nonce::from(n))
    }

    /// Cifra um registro (sem dado associado).
    pub fn seal(&mut self, claro: &[u8]) -> io::Result<Vec<u8>> {
        let nonce = self.nonce()?;
        let cifrado = self
            .cifra
            .encrypt(&nonce, claro)
            .map_err(|_| io::Error::other("não foi possível cifrar o registro"))?;
        self.contador += 1;
        Ok(cifrado)
    }

    /// Decifra um registro.
    pub fn open(&mut self, cifrado: &[u8]) -> io::Result<Vec<u8>> {
        let nonce = self.nonce()?;
        let claro = self.cifra.decrypt(&nonce, cifrado).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "um registro do servidor de controle não passou na conferência",
            )
        })?;
        self.contador += 1;
        Ok(claro)
    }
}

/// O resultado de um handshake: um sentido para mandar e outro para receber.
pub struct Transport {
    /// Cifra o que vai para o outro lado.
    pub tx: Cipher,
    /// Decifra o que vem do outro lado.
    pub rx: Cipher,
}

fn prologo(versao: u16) -> String {
    format!("Tailscale Control Protocol v{versao}")
}

/// A iniciação já enviada, esperando a resposta do servidor.
pub struct Initiation {
    estado: HandshakeState,
}

/// Começa o handshake como cliente. Devolve a mensagem de 101 bytes para mandar.
///
/// `efemera` fixa a chave efêmera e só existe para os testes com vetor.
pub fn start_client(
    maquina: &MachineKey,
    servidor: &MachinePublic,
    versao: u16,
    efemera: Option<&[u8; 32]>,
) -> Result<(Initiation, [u8; TAMANHO_INICIACAO]), NoiseError> {
    let padrao = PADRAO.parse().map_err(erro_snow)?;
    let prologo = prologo(versao);
    let privada = Zeroizing::new(maquina.to_bytes());
    let mut construtor = Builder::new(padrao)
        .prologue(prologo.as_bytes())
        .map_err(erro_snow)?
        .local_private_key(privada.as_ref())
        .map_err(erro_snow)?
        .remote_public_key(servidor.as_bytes())
        .map_err(erro_snow)?;
    if let Some(e) = efemera {
        construtor = construtor.fixed_ephemeral_key_for_testing_only(e);
    }
    let mut estado = construtor.build_initiator().map_err(erro_snow)?;
    let mut corpo = [0u8; CORPO_INICIACAO];
    let n = estado.write_message(&[], &mut corpo).map_err(erro_snow)?;
    if n != CORPO_INICIACAO {
        return Err(NoiseError::Protocol(format!(
            "a iniciação saiu com {n} bytes, esperava {CORPO_INICIACAO}"
        )));
    }
    let mut msg = [0u8; TAMANHO_INICIACAO];
    msg[0..2].copy_from_slice(&versao.to_be_bytes());
    msg[2] = TIPO_INICIACAO;
    msg[3..5].copy_from_slice(&(CORPO_INICIACAO as u16).to_be_bytes());
    msg[5..].copy_from_slice(&corpo);
    Ok((Initiation { estado }, msg))
}

impl Initiation {
    /// Termina o handshake com a resposta de 51 bytes.
    pub fn finish(mut self, resposta: &[u8; TAMANHO_RESPOSTA]) -> Result<Transport, NoiseError> {
        if resposta[0] != TIPO_RESPOSTA
            || u16::from_be_bytes([resposta[1], resposta[2]]) as usize != CORPO_RESPOSTA
        {
            return Err(NoiseError::Protocol(
                "a resposta do handshake tem um cabeçalho inesperado".to_string(),
            ));
        }
        let mut vazio = [0u8; 0];
        self.estado
            .read_message(&resposta[CABECALHO..], &mut vazio)
            .map_err(erro_snow)?;
        let (c1, c2) = self.estado.dangerously_get_raw_split();
        let (c1, c2) = (Zeroizing::new(c1), Zeroizing::new(c2));
        Ok(Transport {
            tx: Cipher::new(&c1),
            rx: Cipher::new(&c2),
        })
    }
}

/// O texto base64 que vai no cabeçalho `X-Tailscale-Handshake`.
pub fn iniciacao_em_base64(msg: &[u8; TAMANHO_INICIACAO]) -> String {
    base64::engine::general_purpose::STANDARD.encode(msg)
}

/// Lê a resposta do handshake de `leitor`: os 51 bytes, ou o erro que o servidor mandou.
pub async fn read_response<R: AsyncRead + Unpin>(
    leitor: &mut R,
) -> Result<[u8; TAMANHO_RESPOSTA], NoiseError> {
    use tokio::io::AsyncReadExt;
    let mut resp = [0u8; TAMANHO_RESPOSTA];
    leitor.read_exact(&mut resp[..CABECALHO]).await?;
    if resp[0] == TIPO_ERRO {
        let tam = u16::from_be_bytes([resp[1], resp[2]]) as usize;
        let mut texto = vec![0u8; tam];
        leitor.read_exact(&mut texto).await?;
        return Err(NoiseError::Server(
            String::from_utf8_lossy(&texto).trim().to_string(),
        ));
    }
    leitor.read_exact(&mut resp[CABECALHO..]).await?;
    Ok(resp)
}

/// O lado do servidor do handshake (só para o plano de controle falso).
#[cfg(any(test, feature = "test-util"))]
pub mod servidor {
    use super::*;

    /// O que o servidor aprendeu da iniciação.
    pub struct Accepted {
        /// A resposta de 51 bytes para mandar.
        pub resposta: [u8; TAMANHO_RESPOSTA],
        /// O canal, do ponto de vista do servidor.
        pub transport: Transport,
        /// A chave pública da máquina que chamou.
        pub maquina: MachinePublic,
        /// A versão de capacidade que o cliente declarou.
        pub versao: u16,
    }

    /// Aceita uma iniciação de 101 bytes.
    pub fn accept(
        servidor: &MachineKey,
        iniciacao: &[u8; TAMANHO_INICIACAO],
        efemera: Option<&[u8; 32]>,
    ) -> Result<Accepted, NoiseError> {
        let versao = u16::from_be_bytes([iniciacao[0], iniciacao[1]]);
        if iniciacao[2] != TIPO_INICIACAO
            || u16::from_be_bytes([iniciacao[3], iniciacao[4]]) as usize != CORPO_INICIACAO
        {
            return Err(NoiseError::Protocol(
                "a iniciação tem um cabeçalho inesperado".to_string(),
            ));
        }
        let padrao = PADRAO.parse().map_err(erro_snow)?;
        let prologo = prologo(versao);
        let privada = Zeroizing::new(servidor.to_bytes());
        let mut construtor = Builder::new(padrao)
            .prologue(prologo.as_bytes())
            .map_err(erro_snow)?
            .local_private_key(privada.as_ref())
            .map_err(erro_snow)?;
        if let Some(e) = efemera {
            construtor = construtor.fixed_ephemeral_key_for_testing_only(e);
        }
        let mut estado = construtor.build_responder().map_err(erro_snow)?;
        let mut vazio = [0u8; 0];
        estado
            .read_message(&iniciacao[5..], &mut vazio)
            .map_err(erro_snow)?;
        let maquina = estado
            .get_remote_static()
            .and_then(|s| <[u8; 32]>::try_from(s).ok())
            .map(MachinePublic::from_bytes)
            .ok_or_else(|| NoiseError::Protocol("a iniciação não trouxe a chave".to_string()))?;
        let mut resposta = [0u8; TAMANHO_RESPOSTA];
        resposta[0] = TIPO_RESPOSTA;
        resposta[1..3].copy_from_slice(&(CORPO_RESPOSTA as u16).to_be_bytes());
        let n = estado
            .write_message(&[], &mut resposta[CABECALHO..])
            .map_err(erro_snow)?;
        debug_assert_eq!(n, CORPO_RESPOSTA);
        let (c1, c2) = estado.dangerously_get_raw_split();
        let (c1, c2) = (Zeroizing::new(c1), Zeroizing::new(c2));
        Ok(Accepted {
            resposta,
            transport: Transport {
                tx: Cipher::new(&c2),
                rx: Cipher::new(&c1),
            },
            maquina,
            versao,
        })
    }

    /// A mensagem de erro de um servidor que recusa o handshake.
    pub fn mensagem_de_erro(texto: &str) -> Vec<u8> {
        let mut m = vec![TIPO_ERRO];
        m.extend_from_slice(&(texto.len() as u16).to_be_bytes());
        m.extend_from_slice(texto.as_bytes());
        m
    }
}

/// Onde a carga antecipada do servidor fica, quando ela chega.
#[derive(Clone, Default)]
pub struct EarlyHandle(Arc<Mutex<Option<EarlyNoise>>>);

impl EarlyHandle {
    /// A carga antecipada, se o servidor mandou uma e ela já foi lida.
    pub fn get(&self) -> Option<EarlyNoise> {
        self.0.lock().ok().and_then(|g| g.clone())
    }

    fn set(&self, e: EarlyNoise) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some(e);
        }
    }
}

/// Em que ponto está a leitura da carga antecipada.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inicio {
    /// Ainda não olhou os primeiros bytes.
    Pendente,
    /// Já passou (com ou sem carga antecipada).
    Resolvido,
}

/// O canal Noise por cima de uma conexão: lê e escreve bytes comuns, e por baixo cifra em
/// registros.
pub struct NoiseStream<S> {
    dentro: S,
    tx: Cipher,
    rx: Cipher,
    // Leitura: bytes cifrados que chegaram e ainda não formam um registro inteiro.
    entrada: Vec<u8>,
    // O texto claro já decifrado e ainda não entregue.
    claro: Vec<u8>,
    claro_pos: usize,
    fim: bool,
    inicio: Inicio,
    antecipada: EarlyHandle,
    // Escrita: o registro cifrado que ainda não foi todo para a conexão.
    saida: Vec<u8>,
    saida_pos: usize,
}

impl<S> NoiseStream<S> {
    fn novo(dentro: S, canal: Transport, detectar_antecipada: bool) -> Self {
        NoiseStream {
            dentro,
            tx: canal.tx,
            rx: canal.rx,
            entrada: Vec::new(),
            claro: Vec::new(),
            claro_pos: 0,
            fim: false,
            inicio: if detectar_antecipada {
                Inicio::Pendente
            } else {
                Inicio::Resolvido
            },
            antecipada: EarlyHandle::default(),
            saida: Vec::new(),
            saida_pos: 0,
        }
    }

    /// O canal do cliente: olha os primeiros bytes do servidor atrás da carga antecipada.
    pub fn client(dentro: S, canal: Transport) -> Self {
        Self::novo(dentro, canal, true)
    }

    /// O canal do servidor (para o plano de controle falso).
    pub fn server(dentro: S, canal: Transport) -> Self {
        Self::novo(dentro, canal, false)
    }

    /// Onde a carga antecipada aparece quando chega. Pode ser clonado antes de entregar o
    /// canal ao HTTP/2.
    pub fn early(&self) -> EarlyHandle {
        self.antecipada.clone()
    }

    fn claro_disponivel(&self) -> usize {
        self.claro.len() - self.claro_pos
    }
}

impl<S: AsyncRead + Unpin> NoiseStream<S> {
    /// Tenta tirar um registro de `entrada` e juntar o texto claro. `Ok(true)` se saiu um.
    fn tirar_registro(&mut self) -> io::Result<bool> {
        if self.entrada.len() < CABECALHO {
            return Ok(false);
        }
        let tipo = self.entrada[0];
        let tam = u16::from_be_bytes([self.entrada[1], self.entrada[2]]) as usize;
        if tipo != TIPO_REGISTRO {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "o servidor de controle mandou uma mensagem do tipo {tipo} no meio da conversa"
                ),
            ));
        }
        if !(ETIQUETA..=MAIOR_CIFRADO).contains(&tam) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("um registro do servidor de controle tem tamanho inválido ({tam})"),
            ));
        }
        if self.entrada.len() < CABECALHO + tam {
            return Ok(false);
        }
        let aberto = self.rx.open(&self.entrada[CABECALHO..CABECALHO + tam])?;
        self.entrada.drain(..CABECALHO + tam);
        if self.claro_pos == self.claro.len() {
            self.claro.clear();
            self.claro_pos = 0;
        }
        self.claro.extend_from_slice(&aberto);
        Ok(true)
    }

    /// Lê da conexão até haver mais um registro de texto claro. `Ready(Ok(false))` no fim.
    fn poll_mais_claro(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<bool>> {
        loop {
            if self.tirar_registro()? {
                return Poll::Ready(Ok(true));
            }
            if self.fim {
                return Poll::Ready(Ok(false));
            }
            let mut bruto = [0u8; 8192];
            let mut buf = ReadBuf::new(&mut bruto);
            match Pin::new(&mut self.dentro).poll_read(cx, &mut buf) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {
                    let lido = buf.filled();
                    if lido.is_empty() {
                        if !self.entrada.is_empty() {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "a conexão com o servidor de controle fechou no meio de um registro",
                            )));
                        }
                        self.fim = true;
                    } else {
                        self.entrada.extend_from_slice(lido);
                    }
                }
            }
        }
    }

    /// Resolve a carga antecipada: olha os 9 primeiros bytes e, se for o caso, consome a carga.
    fn poll_inicio(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.inicio == Inicio::Pendente {
            if self.claro_disponivel() >= INICIO {
                let ini = &self.claro[self.claro_pos..self.claro_pos + INICIO];
                if &ini[..5] != MARCA_ANTECIPADA {
                    self.inicio = Inicio::Resolvido;
                    break;
                }
                let tam = u32::from_be_bytes([ini[5], ini[6], ini[7], ini[8]]) as usize;
                if tam > MAIOR_ANTECIPADA {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "a carga antecipada do servidor de controle é grande demais",
                    )));
                }
                if self.claro_disponivel() >= INICIO + tam {
                    let json = &self.claro[self.claro_pos + INICIO..self.claro_pos + INICIO + tam];
                    let e: EarlyNoise = serde_json::from_slice(json).map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("a carga antecipada do servidor de controle não é válida: {e}"),
                        )
                    })?;
                    self.antecipada.set(e);
                    self.claro_pos += INICIO + tam;
                    self.inicio = Inicio::Resolvido;
                    break;
                }
            }
            match self.poll_mais_claro(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                // Acabou antes dos 9 bytes: o que houver é entregue como está.
                Poll::Ready(Ok(false)) => {
                    self.inicio = Inicio::Resolvido;
                }
                Poll::Ready(Ok(true)) => {}
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for NoiseStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        match this.poll_inicio(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        loop {
            let disponivel = this.claro_disponivel();
            if disponivel > 0 {
                let n = disponivel.min(buf.remaining());
                buf.put_slice(&this.claro[this.claro_pos..this.claro_pos + n]);
                this.claro_pos += n;
                return Poll::Ready(Ok(()));
            }
            match this.poll_mais_claro(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                // Fim da conversa: devolver nada é o fim do fluxo.
                Poll::Ready(Ok(false)) => return Poll::Ready(Ok(())),
                Poll::Ready(Ok(true)) => {}
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> NoiseStream<S> {
    /// Manda para a conexão o registro cifrado que está pendente.
    fn poll_esvaziar(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.saida_pos < self.saida.len() {
            match Pin::new(&mut self.dentro).poll_write(cx, &self.saida[self.saida_pos..]) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "a conexão com o servidor de controle não aceita mais dados",
                    )));
                }
                Poll::Ready(Ok(n)) => self.saida_pos += n,
            }
        }
        self.saida.clear();
        self.saida_pos = 0;
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for NoiseStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        // Um registro de cada vez: o anterior sai antes de cifrar o próximo.
        match this.poll_esvaziar(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        let n = buf.len().min(MAIOR_CLARO);
        let cifrado = this.tx.seal(&buf[..n])?;
        this.saida.push(TIPO_REGISTRO);
        this.saida
            .extend_from_slice(&(cifrado.len() as u16).to_be_bytes());
        this.saida.extend_from_slice(&cifrado);
        // Tenta já; se a conexão estiver cheia, o resto sai no próximo `poll`.
        if let Poll::Ready(Err(e)) = this.poll_esvaziar(cx) {
            return Poll::Ready(Err(e));
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match this.poll_esvaziar(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.dentro).poll_flush(cx),
            outro => outro,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match this.poll_esvaziar(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.dentro).poll_shutdown(cx),
            outro => outro,
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn hex32(s: &str) -> [u8; 32] {
        hex(s).try_into().unwrap()
    }

    // Vetores gerados por uma implementação independente do Noise IK (em Python, com
    // X25519, BLAKE2s e ChaCha20-Poly1305 de bibliotecas comuns), a partir do que o
    // protocolo fixa: prólogo "Tailscale Control Protocol v113", pré-mensagem com a chave do
    // servidor, chaves 0x01 (máquina), 0x02 (servidor), 0x03 e 0x04 (efêmeras).
    const VERSAO: u16 = 113;
    const MSG1: &str = "00710100605dfedd3b6bd47f6fa28ee15d969d5bb0ea53774d488bdaf9df1c6e0124b3ef224391bcfef3f1b0f051873c2103356110f8056ef928c4354783347c74dc7b71b7a9b7da1f40c3a7cd633136e4132f8791b1bdba20e36b9492f17f73a947cf794a";
    const MSG2: &str = "020030ac01b2209e86354fb853237b5de0f4fab13c7fcbf433a61c019369617fecf10b41c33a53a8dcf620bab53dcab6be9b14";
    const C1: &str = "8b63a17b3027c853ffcfea99dae9d090a15504924d85f87dedb5f6f99e53f853";
    const C2: &str = "9b40738cc543c05f5ebd16dac9b12211c6e9edbb42385d92c47bd6def1f318ae";
    const SERVIDOR_PUB: &str = "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59";
    // O registro 0 e 1 e o 258 de "C1", para travar o nonce big-endian.
    const REG0: &str = "79930a5099faba342deeeac1184f7a83ede052de1bdce17890ae6af0ee";
    const REG1: &str = "bb84be033574f5973e65bf25034568dbfa8741d21fe208";
    const REG258: &str = "c763021ef7e141217fd2996bfdf9730360194630c6c58d5570b82cdba16c9f558191a2";

    fn chaves() -> (MachineKey, MachineKey) {
        (
            MachineKey::from_bytes([1; 32]),
            MachineKey::from_bytes([2; 32]),
        )
    }

    #[test]
    fn a_iniciacao_bate_com_o_vetor() {
        let (m, s) = chaves();
        assert_eq!(s.public().to_string(), format!("mkey:{SERVIDOR_PUB}"));
        let (_, msg) = start_client(&m, &s.public(), VERSAO, Some(&[3; 32])).unwrap();
        assert_eq!(msg.len(), 101);
        assert_eq!(msg.to_vec(), hex(MSG1));
        // O cabeçalho: versão 113 em dois bytes, tipo 1, tamanho 96.
        assert_eq!(&msg[..5], &[0, 113, 1, 0, 96]);
    }

    #[test]
    fn a_resposta_do_vetor_da_as_chaves_do_vetor() {
        let (m, s) = chaves();
        let (inic, _) = start_client(&m, &s.public(), VERSAO, Some(&[3; 32])).unwrap();
        let resp: [u8; TAMANHO_RESPOSTA] = hex(MSG2).try_into().unwrap();
        let mut canal = inic.finish(&resp).unwrap();
        // O cliente cifra com a primeira chave: o registro 0 do vetor é o que ele manda.
        assert_eq!(canal.tx.seal(b"ola, controle").unwrap(), hex(REG0));
        assert_eq!(canal.tx.seal(b"segundo").unwrap(), hex(REG1));
        // E decifra com a segunda.
        let mut do_servidor = Cipher::new(&hex32(C2));
        let cifrado = do_servidor.seal(b"resposta").unwrap();
        assert_eq!(canal.rx.open(&cifrado).unwrap(), b"resposta");
        // Para a conferência direta da chave C1.
        let mut c1 = Cipher::new(&hex32(C1));
        assert_eq!(c1.seal(b"ola, controle").unwrap(), hex(REG0));
    }

    #[test]
    fn o_servidor_responde_com_o_vetor() {
        let (_, s) = chaves();
        let init: [u8; TAMANHO_INICIACAO] = hex(MSG1).try_into().unwrap();
        let aceito = servidor::accept(&s, &init, Some(&[4; 32])).unwrap();
        assert_eq!(aceito.resposta.to_vec(), hex(MSG2));
        assert_eq!(aceito.versao, 113);
        assert_eq!(aceito.maquina, MachineKey::from_bytes([1; 32]).public());
        // O servidor decifra o que o cliente cifrou com C1.
        let mut canal = aceito.transport;
        assert_eq!(canal.rx.open(&hex(REG0)).unwrap(), b"ola, controle");
    }

    #[test]
    fn o_nonce_e_big_endian() {
        let mut c = Cipher::new(&hex32(C1));
        c.contador = 258;
        assert_eq!(c.seal(b"nonce em big-endian").unwrap(), hex(REG258));
        // Em little-endian o resultado seria outro: o contador 258 é 00 00 00 00 00 00 01 02.
        let mut r = Cipher::new(&hex32(C1));
        r.contador = 258;
        assert_eq!(r.open(&hex(REG258)).unwrap(), b"nonce em big-endian");
    }

    #[test]
    fn o_ultimo_nonce_e_invalido() {
        let mut c = Cipher::new(&[7; 32]);
        c.contador = u64::MAX;
        assert!(c.seal(b"x").is_err());
        assert!(c.open(&[0; 20]).is_err());
    }

    #[test]
    fn a_chave_errada_do_servidor_nao_fecha_o_handshake() {
        let (m, s) = chaves();
        let outra = MachineKey::from_bytes([9; 32]);
        // O cliente acha que o servidor é `outra`; o servidor de verdade é `s`.
        let (_, msg) = start_client(&m, &outra.public(), VERSAO, None).unwrap();
        assert!(servidor::accept(&s, &msg, None).is_err());
    }

    #[test]
    fn versao_diferente_muda_o_prologo_e_o_handshake_falha() {
        let (m, s) = chaves();
        let (_, mut msg) = start_client(&m, &s.public(), 113, None).unwrap();
        msg[0..2].copy_from_slice(&114u16.to_be_bytes());
        assert!(servidor::accept(&s, &msg, None).is_err());
    }

    #[test]
    fn resposta_corrompida_falha() {
        let (m, s) = chaves();
        let (inic, msg) = start_client(&m, &s.public(), VERSAO, None).unwrap();
        let aceito = servidor::accept(&s, &msg, None).unwrap();
        let mut resp = aceito.resposta;
        resp[40] ^= 1;
        assert!(inic.finish(&resp).is_err());
    }

    /// Faz um par cliente/servidor sobre um `duplex` em memória.
    async fn par() -> (
        NoiseStream<tokio::io::DuplexStream>,
        NoiseStream<tokio::io::DuplexStream>,
    ) {
        let (m, s) = chaves();
        let (a, b) = duplex(64 * 1024);
        let (inic, msg) = start_client(&m, &s.public(), VERSAO, None).unwrap();
        let aceito = servidor::accept(&s, &msg, None).unwrap();
        let cliente = inic.finish(&aceito.resposta).unwrap();
        (
            NoiseStream::client(a, cliente),
            NoiseStream::server(b, aceito.transport),
        )
    }

    #[tokio::test]
    async fn os_bytes_vao_e_voltam() {
        let (mut c, mut s) = par().await;
        c.write_all(b"ola").await.unwrap();
        c.flush().await.unwrap();
        let mut b = [0u8; 3];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"ola");
        s.write_all(b"oi de volta").await.unwrap();
        s.flush().await.unwrap();
        let mut b = [0u8; 11];
        c.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"oi de volta");
    }

    #[tokio::test]
    async fn um_texto_grande_vira_varios_registros() {
        let (mut c, mut s) = par().await;
        let dados: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let esperado = dados.clone();
        let envio = tokio::spawn(async move {
            c.write_all(&dados).await.unwrap();
            c.flush().await.unwrap();
            c
        });
        let mut recebido = vec![0u8; esperado.len()];
        s.read_exact(&mut recebido).await.unwrap();
        assert_eq!(recebido, esperado);
        envio.await.unwrap();
    }

    #[tokio::test]
    async fn nenhum_registro_passa_de_4096_bytes() {
        let (m, srv) = chaves();
        let (a, mut b) = duplex(1 << 20);
        let (inic, msg) = start_client(&m, &srv.public(), VERSAO, None).unwrap();
        let aceito = servidor::accept(&srv, &msg, None).unwrap();
        let mut c = NoiseStream::client(a, inic.finish(&aceito.resposta).unwrap());
        c.write_all(&vec![1u8; 10_000]).await.unwrap();
        c.flush().await.unwrap();
        drop(c);
        let mut bruto = Vec::new();
        b.read_to_end(&mut bruto).await.unwrap();
        let mut pos = 0;
        let mut registros = 0;
        let mut total_claro = 0;
        while pos < bruto.len() {
            assert_eq!(bruto[pos], 4);
            let tam = u16::from_be_bytes([bruto[pos + 1], bruto[pos + 2]]) as usize;
            assert!(3 + tam <= 4096, "registro de {} bytes", 3 + tam);
            total_claro += tam - 16;
            pos += 3 + tam;
            registros += 1;
        }
        assert_eq!(pos, bruto.len());
        assert_eq!(total_claro, 10_000);
        // 4077 + 4077 + 1846.
        assert_eq!(registros, 3);
    }

    #[tokio::test]
    async fn sem_carga_antecipada_os_nove_bytes_ficam_para_o_http2() {
        let (mut c, mut s) = par().await;
        // O começo de um quadro SETTINGS do HTTP/2 (9 bytes) e mais alguma coisa.
        let settings = [0, 0, 0, 4, 0, 0, 0, 0, 0, 9, 9];
        s.write_all(&settings).await.unwrap();
        s.flush().await.unwrap();
        let mut b = [0u8; 11];
        c.read_exact(&mut b).await.unwrap();
        assert_eq!(b, settings);
        assert!(c.early().get().is_none());
    }

    #[tokio::test]
    async fn a_carga_antecipada_e_lida_e_o_resto_segue() {
        let (mut c, mut s) = par().await;
        let desafio = crate::key::ChallengePublic::from_bytes([5; 32]);
        let json = format!("{{\"nodeKeyChallenge\":\"{desafio}\"}}");
        let mut quadro = MARCA_ANTECIPADA.to_vec();
        quadro.extend_from_slice(&(json.len() as u32).to_be_bytes());
        quadro.extend_from_slice(json.as_bytes());
        quadro.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 0, 0]);
        // Em dois pedaços, para exercitar a espera.
        s.write_all(&quadro[..12]).await.unwrap();
        s.flush().await.unwrap();
        s.write_all(&quadro[12..]).await.unwrap();
        s.flush().await.unwrap();
        let alca = c.early();
        let mut b = [0u8; 9];
        c.read_exact(&mut b).await.unwrap();
        assert_eq!(b, [0, 0, 0, 4, 0, 0, 0, 0, 0]);
        assert_eq!(alca.get().unwrap().node_key_challenge, desafio);
    }

    #[tokio::test]
    async fn carga_antecipada_com_json_ruim_da_erro() {
        let (mut c, mut s) = par().await;
        let mut quadro = MARCA_ANTECIPADA.to_vec();
        quadro.extend_from_slice(&3u32.to_be_bytes());
        quadro.extend_from_slice(b"{x}");
        s.write_all(&quadro).await.unwrap();
        s.flush().await.unwrap();
        let mut b = [0u8; 1];
        let e = c.read_exact(&mut b).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn registro_adulterado_da_erro() {
        let (m, srv) = chaves();
        let (a, mut b) = duplex(4096);
        let (inic, msg) = start_client(&m, &srv.public(), VERSAO, None).unwrap();
        let aceito = servidor::accept(&srv, &msg, None).unwrap();
        let mut c = NoiseStream::client(a, inic.finish(&aceito.resposta).unwrap());
        let mut s = NoiseStream::server(tokio::io::empty(), aceito.transport);
        // Cifra um registro com o servidor "de mentira" e adultera.
        let mut cifrado = s.tx.seal(b"0123456789abcdef0123").unwrap();
        cifrado[0] ^= 1;
        let mut bruto = vec![4];
        bruto.extend_from_slice(&(cifrado.len() as u16).to_be_bytes());
        bruto.extend_from_slice(&cifrado);
        b.write_all(&bruto).await.unwrap();
        let mut x = [0u8; 4];
        let e = c.read_exact(&mut x).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn mensagem_de_erro_do_servidor_vira_erro() {
        let (mut a, mut b) = duplex(1024);
        b.write_all(&servidor::mensagem_de_erro("versão não suportada"))
            .await
            .unwrap();
        match read_response(&mut a).await {
            Err(NoiseError::Server(t)) => assert_eq!(t, "versão não suportada"),
            outro => panic!("esperava erro do servidor, veio {:?}", outro.err()),
        }
    }

    #[tokio::test]
    async fn fim_da_conexao_no_meio_de_um_registro_da_erro() {
        let (m, srv) = chaves();
        let (a, mut b) = duplex(4096);
        let (inic, msg) = start_client(&m, &srv.public(), VERSAO, None).unwrap();
        let aceito = servidor::accept(&srv, &msg, None).unwrap();
        let mut c = NoiseStream::client(a, inic.finish(&aceito.resposta).unwrap());
        b.write_all(&[4, 0, 40, 1, 2, 3]).await.unwrap();
        drop(b);
        let mut x = [0u8; 4];
        let e = c.read_exact(&mut x).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn o_base64_da_iniciacao_tem_136_caracteres() {
        let (m, s) = chaves();
        let (_, msg) = start_client(&m, &s.public(), VERSAO, None).unwrap();
        let b64 = iniciacao_em_base64(&msg);
        // 101 bytes em base64 com preenchimento: ceil(101/3)*4 = 136.
        assert_eq!(b64.len(), 136);
    }
}
