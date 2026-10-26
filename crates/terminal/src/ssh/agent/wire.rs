//! O formato dos pedidos e respostas do agente (draft-miller-ssh-agent): inteiros de 32 bits em
//! ordem de rede e `string`s com o tamanho na frente.

/// Maior moldura aceita (como o `russh`): uma maior fecha a conexão daquele cliente.
pub(super) const MAX_FRAME: usize = 256 * 1024;

pub(super) const FAILURE: u8 = 5;
pub(super) const SUCCESS: u8 = 6;
pub(super) const REQUEST_IDENTITIES: u8 = 11;
pub(super) const IDENTITIES_ANSWER: u8 = 12;
pub(super) const SIGN_REQUEST: u8 = 13;
pub(super) const SIGN_RESPONSE: u8 = 14;
pub(super) const ADD_IDENTITY: u8 = 17;
pub(super) const REMOVE_IDENTITY: u8 = 18;
pub(super) const REMOVE_ALL_IDENTITIES: u8 = 19;
pub(super) const LOCK: u8 = 22;
pub(super) const UNLOCK: u8 = 23;
pub(super) const ADD_ID_CONSTRAINED: u8 = 25;
pub(super) const EXTENSION: u8 = 27;

/// As restrições de `ADD_ID_CONSTRAINED`.
pub(super) const CONSTRAIN_LIFETIME: u8 = 1;
pub(super) const CONSTRAIN_CONFIRM: u8 = 2;

/// Um leitor sobre os bytes de um pedido.
pub(super) struct Reader<'a>(pub &'a [u8]);

impl<'a> Reader<'a> {
    pub fn u8(&mut self) -> Option<u8> {
        let (primeiro, resto) = self.0.split_first()?;
        self.0 = resto;
        Some(*primeiro)
    }

    pub fn u32(&mut self) -> Option<u32> {
        let (numero, resto) = self.0.split_first_chunk::<4>()?;
        self.0 = resto;
        Some(u32::from_be_bytes(*numero))
    }

    pub fn string(&mut self) -> Option<&'a [u8]> {
        let tamanho = self.u32()? as usize;
        if tamanho > self.0.len() {
            return None;
        }
        let (texto, resto) = self.0.split_at(tamanho);
        self.0 = resto;
        Some(texto)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Um escritor de respostas.
#[derive(Default)]
pub(super) struct Writer(pub Vec<u8>);

impl Writer {
    pub fn new(tipo: u8) -> Self {
        Self(vec![tipo])
    }

    pub fn u32(&mut self, numero: u32) -> &mut Self {
        self.0.extend_from_slice(&numero.to_be_bytes());
        self
    }

    pub fn string(&mut self, texto: &[u8]) -> &mut Self {
        self.u32(texto.len() as u32);
        self.0.extend_from_slice(texto);
        self
    }

    pub fn u8(&mut self, byte: u8) -> &mut Self {
        self.0.push(byte);
        self
    }
}

/// A resposta de um byte só (`SUCCESS` ou `FAILURE`).
pub(super) fn simple(tipo: u8) -> Vec<u8> {
    vec![tipo]
}

/// Põe o tamanho na frente de uma mensagem, pronta para o socket.
pub(super) fn framed(mensagem: &[u8]) -> Vec<u8> {
    let mut quadro = Vec::with_capacity(4 + mensagem.len());
    quadro.extend_from_slice(&(mensagem.len() as u32).to_be_bytes());
    quadro.extend_from_slice(mensagem);
    quadro
}
