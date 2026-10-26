//! O aperto de mão: a máquina de estados do Noise IX com a carga do Nebula (o certificado, os
//! índices e a hora).
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, pacote `handshake`: `machine.go`,
//! `payload.go`), com as mesmas regras do que é fatal e do que se descarta sem estragar o
//! estado: um pacote que o Noise não lê (lixo, uma retransmissão de outro aperto) não mexe na
//! máquina; depois que o Noise avançou, qualquer erro a encerra.

use std::fmt;
use std::sync::Arc;

use crate::cert::{Certificate, Curve, Version};
use crate::config::Credentials;
use crate::header::{self, Header, MessageType};
use crate::noise::{DataCipher, Ix, NoiseError};
use crate::proto::{Reader, Value, Writer};

/// Os campos do `NebulaHandshakeDetails`, no protocolo.
const FIELD_CERT: u32 = 1;
const FIELD_INITIATOR_INDEX: u32 = 2;
const FIELD_RESPONDER_INDEX: u32 = 3;
const FIELD_TIME: u32 = 5;
const FIELD_CERT_VERSION: u32 = 8;

/// O que as duas mensagens do aperto de mão levam (o `NebulaHandshake{Details}`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Payload {
    /// O certificado, sem a chave pública (que vem do Noise).
    pub(crate) cert: Vec<u8>,
    /// O índice de quem iniciou.
    pub(crate) initiator_index: u32,
    /// O índice de quem respondeu.
    pub(crate) responder_index: u32,
    /// A hora de quem escreveu a mensagem, em nanossegundos desde 1970.
    pub(crate) time: u64,
    /// A versão do certificado.
    pub(crate) cert_version: u32,
}

impl Payload {
    /// A codificação do protobuf: campos zerados ou vazios não saem.
    pub(crate) fn marshal(&self) -> Vec<u8> {
        let mut detalhes = Writer::new();
        detalhes.bytes(FIELD_CERT, &self.cert);
        detalhes.uint(FIELD_INITIATOR_INDEX, u64::from(self.initiator_index));
        detalhes.uint(FIELD_RESPONDER_INDEX, u64::from(self.responder_index));
        detalhes.uint(FIELD_TIME, self.time);
        detalhes.uint(FIELD_CERT_VERSION, u64::from(self.cert_version));
        let mut fora = Writer::new();
        fora.bytes_always(1, &detalhes.finish());
        fora.finish()
    }

    /// Lê a carga com a mesma rigidez do Nebula: um campo conhecido com o tipo errado é erro,
    /// o campo repetido vale o último e os desconhecidos são ignorados.
    pub(crate) fn unmarshal(bytes: &[u8]) -> Result<Payload, HandshakeError> {
        let mut carga = Payload::default();
        for item in Reader::new(bytes) {
            let (campo, valor) = item.map_err(|()| HandshakeError::fatal("mensagem malformada"))?;
            if campo == 1
                && let Value::Bytes(detalhes) = valor
            {
                carga.read_details(detalhes)?;
            }
        }
        Ok(carga)
    }

    fn read_details(&mut self, bytes: &[u8]) -> Result<(), HandshakeError> {
        let invalido = || HandshakeError::fatal("detalhes do aperto de mão malformados");
        for item in Reader::new(bytes) {
            let (campo, valor) = item.map_err(|()| invalido())?;
            match campo {
                FIELD_CERT => self.cert = valor.bytes().ok_or_else(invalido)?.to_vec(),
                FIELD_INITIATOR_INDEX => {
                    self.initiator_index = u32::try_from(valor.varint().ok_or_else(invalido)?)
                        .map_err(|_| invalido())?;
                }
                FIELD_RESPONDER_INDEX => {
                    self.responder_index = u32::try_from(valor.varint().ok_or_else(invalido)?)
                        .map_err(|_| invalido())?;
                }
                FIELD_TIME => self.time = valor.varint().ok_or_else(invalido)?,
                FIELD_CERT_VERSION => {
                    self.cert_version = u32::try_from(valor.varint().ok_or_else(invalido)?)
                        .map_err(|_| invalido())?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Por que um pacote do aperto de mão foi recusado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HandshakeError {
    /// `true`: a máquina não serve mais e o aperto de mão acabou. `false`: o pacote foi
    /// descartado e a máquina segue como estava.
    pub(crate) fatal: bool,
    /// O motivo, em português (a frase continua depois de "recusou o handshake: ").
    pub(crate) reason: String,
}

impl HandshakeError {
    fn fatal(motivo: impl Into<String>) -> HandshakeError {
        HandshakeError {
            fatal: true,
            reason: motivo.into(),
        }
    }

    fn ignored(motivo: impl Into<String>) -> HandshakeError {
        HandshakeError {
            fatal: false,
            reason: motivo.into(),
        }
    }
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for HandshakeError {}

/// O aperto de mão que terminou.
pub(crate) struct Completed {
    /// A cifra dos pacotes que enviamos.
    pub(crate) send: DataCipher,
    /// A cifra dos pacotes que recebemos.
    pub(crate) recv: DataCipher,
    /// O nosso certificado que o outro lado viu.
    pub(crate) my_cert: Certificate,
    /// O certificado do outro nó, já conferido.
    pub(crate) remote_cert: Certificate,
    /// O índice do outro lado (vai no cabeçalho do que enviamos).
    pub(crate) remote_index: u32,
    /// O nosso índice (o outro lado o põe no cabeçalho do que nos envia).
    pub(crate) local_index: u32,
    /// A hora do aperto de mão, no relógio do outro lado, em nanossegundos desde 1970.
    pub(crate) time: u64,
    /// Quantas mensagens o aperto de mão usou: os contadores do transporte começam depois.
    pub(crate) message_index: u64,
    /// Fomos nós que iniciamos.
    pub(crate) initiator: bool,
}

/// O resultado de um pacote lido.
pub(crate) struct Step {
    /// A resposta a enviar (o pacote inteiro, com o cabeçalho), se há.
    pub(crate) reply: Option<Vec<u8>>,
    /// O aperto de mão terminou.
    pub(crate) done: Option<Completed>,
}

/// A máquina de um aperto de mão, do lado que inicia ou do que responde.
pub(crate) struct Machine {
    creds: Arc<Credentials>,
    noise: Ix,
    initiator: bool,
    my_version: Version,
    local_index: u32,
    remote_index: u32,
    my_cert: Option<Certificate>,
    remote_cert: Option<Certificate>,
    time: u64,
    initiated: bool,
    failed: bool,
}

impl Machine {
    /// Uma máquina nova. `local_index` é o índice que o engine reservou para este túnel (nunca
    /// zero). Quem inicia usa a versão de certificado `version` (ou a do config, se `None`);
    /// quem responde começa pela do config e troca pela do outro lado se tem um certificado
    /// dela.
    pub(crate) fn new(
        creds: Arc<Credentials>,
        initiator: bool,
        local_index: u32,
        version: Option<Version>,
    ) -> Machine {
        let my_version = version
            .filter(|versao| creds.cert(*versao).is_some())
            .unwrap_or_else(|| creds.default_cert().version());
        let secret = creds.key.secret();
        let mut static_secret = [0u8; 32];
        static_secret.copy_from_slice(secret);
        // A chave privada vem de um `PrivateKey` conferido: a pública sai dela.
        let static_public = creds.key.public_key().unwrap_or([0; 32]);
        let noise = Ix::new(creds.cipher, initiator, static_secret, static_public, &[]);
        Machine {
            creds,
            noise,
            initiator,
            my_version,
            local_index,
            remote_index: 0,
            my_cert: None,
            remote_cert: None,
            time: 0,
            initiated: false,
            failed: false,
        }
    }

    /// A máquina não serve mais.
    #[allow(dead_code)] // quem usa a máquina descarta-a pelo erro fatal
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    /// A versão do certificado com que esta máquina fala.
    #[allow(dead_code)]
    pub(crate) fn version(&self) -> Version {
        self.my_version
    }

    /// A primeira mensagem (só de quem inicia, uma vez). `now` são nanossegundos desde 1970.
    pub(crate) fn initiate(&mut self, now: u64) -> Result<Vec<u8>, HandshakeError> {
        if self.failed {
            return Err(HandshakeError::fatal("o aperto de mão já falhou"));
        }
        if !self.initiator || self.initiated {
            self.failed = true;
            return Err(HandshakeError::fatal(
                "o aperto de mão não pode ser iniciado",
            ));
        }
        self.initiated = true;
        let pacote = self.write(now).inspect_err(|_| self.failed = true)?;
        Ok(pacote)
    }

    /// Escreve a próxima mensagem do Noise com a nossa carga e põe o cabeçalho.
    fn write(&mut self, now: u64) -> Result<Vec<u8>, HandshakeError> {
        let versao = self.my_version;
        let cert = self.creds.cert(versao).ok_or_else(|| {
            HandshakeError::fatal(format!(
                "não há certificado da versão {} para o aperto de mão",
                versao.number()
            ))
        })?;
        let carga = Payload {
            cert: cert.marshal_for_handshake(),
            initiator_index: if self.initiator {
                self.local_index
            } else {
                self.remote_index
            },
            responder_index: if self.initiator { 0 } else { self.local_index },
            time: now,
            cert_version: u32::from(versao.number()),
        };
        let my_cert = cert.clone();
        let mensagem = self
            .noise
            .write_message(&carga.marshal())
            .map_err(|erro| HandshakeError::fatal(erro.to_string()))?;
        self.my_cert = Some(my_cert);
        let contador = self.noise.message_index() as u64;
        let mut pacote = Vec::with_capacity(header::LEN + mensagem.len());
        pacote.extend_from_slice(&header::encode(
            MessageType::Handshake,
            header::HANDSHAKE_IX_PSK0,
            self.remote_index,
            contador,
        ));
        pacote.extend_from_slice(&mensagem);
        Ok(pacote)
    }

    /// Processa um pacote do aperto de mão (com o cabeçalho). `now` são nanossegundos desde 1970.
    pub(crate) fn process(&mut self, packet: &[u8], now: u64) -> Result<Step, HandshakeError> {
        if self.failed {
            return Err(HandshakeError::fatal("o aperto de mão já falhou"));
        }
        let Some(cabecalho) = Header::parse(packet) else {
            return Err(HandshakeError::ignored("o pacote é curto demais"));
        };
        if cabecalho.subtype != header::HANDSHAKE_IX_PSK0 {
            return Err(HandshakeError::ignored(
                "o subtipo do pacote não é o do aperto de mão desta conexão",
            ));
        }
        if self.initiator && !self.initiated {
            self.failed = true;
            return Err(HandshakeError::fatal(
                "o aperto de mão de quem inicia precisa começar antes de receber",
            ));
        }
        // O Noise desfaz o que fez se a mensagem não confere: um lixo não estraga a máquina.
        let carga = self
            .noise
            .read_message(&packet[header::LEN..])
            .map_err(|erro| HandshakeError::ignored(erro.to_string()))?;

        // A partir daqui o Noise avançou, e qualquer erro encerra o aperto de mão.
        match self.after_read(&carga, now) {
            Ok(passo) => Ok(passo),
            Err(erro) => {
                self.failed = true;
                Err(erro)
            }
        }
    }

    fn after_read(&mut self, carga: &[u8], now: u64) -> Result<Step, HandshakeError> {
        if carga.is_empty() {
            return Err(HandshakeError::fatal(
                "a mensagem veio sem o certificado nem os índices",
            ));
        }
        let carga = Payload::unmarshal(carga)?;
        // Nas duas mensagens do IX vêm os índices e o certificado.
        let tem_indices =
            carga.initiator_index != 0 || carga.responder_index != 0 || carga.time != 0;
        if !tem_indices || carga.cert.is_empty() {
            return Err(HandshakeError::fatal(
                "a mensagem veio sem o certificado ou sem os índices",
            ));
        }
        let remoto = if self.initiator {
            carga.responder_index
        } else {
            carga.initiator_index
        };
        if remoto == 0 {
            return Err(HandshakeError::fatal(
                "o outro nó mandou um índice inválido",
            ));
        }
        self.remote_index = remoto;
        self.time = carga.time;
        self.validate_cert(&carga, now)?;

        if self.initiator {
            // A resposta do respondedor termina o aperto de mão.
            let divisao = self
                .noise
                .split()
                .ok_or_else(|| HandshakeError::fatal("o aperto de mão não terminou"))?;
            return Ok(Step {
                reply: None,
                done: Some(self.completed(
                    DataCipher::new(self.creds.cipher, &divisao.initiator_to_responder),
                    DataCipher::new(self.creds.cipher, &divisao.responder_to_initiator),
                )?),
            });
        }
        // O respondedor escreve a segunda mensagem e termina com ela.
        let resposta = self.write(now)?;
        let divisao = self
            .noise
            .split()
            .ok_or_else(|| HandshakeError::fatal("o aperto de mão não terminou"))?;
        let completo = self.completed(
            DataCipher::new(self.creds.cipher, &divisao.responder_to_initiator),
            DataCipher::new(self.creds.cipher, &divisao.initiator_to_responder),
        )?;
        Ok(Step {
            reply: Some(resposta),
            done: Some(completo),
        })
    }

    fn completed(
        &mut self,
        send: DataCipher,
        recv: DataCipher,
    ) -> Result<Completed, HandshakeError> {
        let (Some(my_cert), Some(remote_cert)) = (self.my_cert.take(), self.remote_cert.take())
        else {
            return Err(HandshakeError::fatal(
                "o aperto de mão terminou sem receber o que precisava",
            ));
        };
        Ok(Completed {
            send,
            recv,
            my_cert,
            remote_cert,
            remote_index: self.remote_index,
            local_index: self.local_index,
            time: self.time,
            message_index: self.noise.message_index() as u64,
            initiator: self.initiator,
        })
    }

    fn validate_cert(&mut self, carga: &Payload, now: u64) -> Result<(), HandshakeError> {
        let Some(versao) = u8::try_from(carga.cert_version)
            .ok()
            .and_then(|n| Version::from_number(u32::from(n)))
        else {
            return Err(HandshakeError::fatal(format!(
                "versão de certificado desconhecida ({})",
                carga.cert_version
            )));
        };
        let chave = self
            .noise
            .remote_static()
            .ok_or_else(|| HandshakeError::fatal("o Noise não trouxe a chave do outro nó"))?;
        let curva: Curve = self.creds.key.curve();
        let remoto = Certificate::unmarshal(versao, &carga.cert, Some(&chave), curva)
            .map_err(|erro| HandshakeError::fatal(format!("certificado inválido: {erro}")))?;
        if remoto.public_key() != chave {
            return Err(HandshakeError::fatal(
                "a chave do certificado não é a do aperto de mão",
            ));
        }
        // Se o outro lado fala numa versão que também temos, passamos a falar nela.
        if remoto.version() != self.my_version && self.creds.cert(remoto.version()).is_some() {
            self.my_version = remoto.version();
        }
        let segundos = i64::try_from(now / 1_000_000_000).unwrap_or(i64::MAX);
        self.creds
            .pool
            .verify(&remoto, segundos)
            .map_err(|erro| HandshakeError::fatal(erro.to_string()))?;

        if remoto.networks().is_empty() {
            return Err(HandshakeError::fatal(
                "o certificado do outro nó não tem nenhuma rede",
            ));
        }
        let meus = self.creds.default_cert().networks();
        if remoto
            .networks()
            .iter()
            .any(|rede| meus.iter().any(|minha| minha.addr() == rede.addr()))
        {
            return Err(HandshakeError::fatal(
                "o outro nó tem o mesmo endereço do nosso (aperto de mão com o próprio nó)",
            ));
        }
        self.remote_cert = Some(remoto);
        Ok(())
    }
}

impl From<NoiseError> for HandshakeError {
    fn from(erro: NoiseError) -> Self {
        HandshakeError::ignored(erro.to_string())
    }
}

#[cfg(test)]
mod tests;
