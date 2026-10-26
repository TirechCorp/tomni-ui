//! O pacote do VL1: cabeçalho, verbos, fragmentos e compressão LZ4.
//!
//! O formato (`Packet.hpp`): 8 bytes de ID/IV, destino (5), origem (5), flags (1), MAC (8), verbo
//! (1) e a carga. Nas flags, do bit mais alto ao mais baixo: `E` armadura estendida, `F`
//! fragmentado, `CCC` suíte de cifra e `HHH` saltos.

use crate::address::{Address, RESERVED_PREFIX};
use crate::{Error, Result};

/// Onde ficam os campos do cabeçalho do pacote (`ZT_PACKET_IDX_*`, `Packet.hpp`).
pub mod idx {
    /// ID do pacote, usado também como IV da cifra (8 bytes).
    pub const IV: usize = 0;
    /// Endereço do destino (5 bytes).
    pub const DEST: usize = 8;
    /// Endereço da origem (5 bytes).
    pub const SOURCE: usize = 13;
    /// Flags: armadura estendida, fragmentado, suíte de cifra e saltos.
    pub const FLAGS: usize = 18;
    /// MAC de 8 bytes (ou o resto da etiqueta do AES-GMAC-SIV).
    pub const MAC: usize = 19;
    /// O verbo, com o bit de "comprimido" no alto.
    pub const VERB: usize = 27;
    /// O começo da carga do verbo.
    pub const PAYLOAD: usize = 28;
}

/// Tamanho do cabeçalho, ou seja, o menor pacote possível (`ZT_PROTO_MIN_PACKET_LENGTH`).
pub const MIN_PACKET_LEN: usize = idx::PAYLOAD;

/// Bit das flags: a armadura estendida cobre o pacote (`ZT_PROTO_FLAG_EXTENDED_ARMOR`).
pub const FLAG_EXTENDED_ARMOR: u8 = 0x80;
/// Bit das flags: há fragmentos a seguir (`ZT_PROTO_FLAG_FRAGMENTED`).
pub const FLAG_FRAGMENTED: u8 = 0x40;
/// Máscara dos três bits de saltos nas flags.
pub const MASK_HOPS: u8 = 0x07;
/// Máscara dos três bits da suíte de cifra nas flags.
pub const MASK_CIPHER: u8 = 0x38;
/// Bit do verbo: a carga está comprimida com LZ4 (`ZT_PROTO_VERB_FLAG_COMPRESSED`).
pub const VERB_FLAG_COMPRESSED: u8 = 0x80;
/// Máscara do número do verbo.
pub const MASK_VERB: u8 = 0x1f;

/// Máximo de saltos de um pacote por nós intermediários (`ZT_PROTO_MAX_HOPS`).
pub const MAX_HOPS: u8 = 7;

/// MTU de carga UDP que o protocolo supõe para dividir em fragmentos
/// (`ZT_DEFAULT_PHYSMTU`, `ZeroTierOne.h`).
pub const UDP_PAYLOAD_MTU: usize = 1432;

/// Máximo de pedaços que este nó monta e manda (`ZT_MAX_PACKET_FRAGMENTS`, `Constants.hpp`).
pub const MAX_FRAGMENTS: usize = 7;

/// O máximo que cabe no campo do fragmento: o total ocupa 4 bits (`Packet.hpp`, comentário de
/// `Fragment`).
const MAX_FRAGMENTS_NO_FIO: usize = 16;

/// Maior pacote inteiro, antes de dividir (`ZT_PROTO_MAX_PACKET_LENGTH`, `Packet.hpp`).
pub const MAX_PACKET_LEN: usize = MAX_FRAGMENTS * UDP_PAYLOAD_MTU;

/// Tamanho do cabeçalho de um fragmento (`ZT_PROTO_MIN_FRAGMENT_LENGTH`).
pub const FRAGMENT_HEADER_LEN: usize = 16;

/// Onde ficam os campos do fragmento (`ZT_PACKET_FRAGMENT_IDX_*`).
mod fragmento {
    pub(super) const INDICADOR: usize = 13;
    pub(super) const NUMERO: usize = 14;
    pub(super) const SALTOS: usize = 15;
}

/// A suíte de cifra de um pacote (`ZT_PROTO_CIPHER_SUITE__*`, `Packet.hpp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    /// Poly1305 sem cifrar a carga (o HELLO, que ainda não tem chave).
    None,
    /// Salsa20/12 + Poly1305.
    Salsa2012,
    /// Sem cifra, num caminho de confiança (não suportado).
    TrustedPath,
    /// AES-GMAC-SIV (protocolo 12 em diante).
    AesGmacSiv,
    /// Valor que o protocolo não define.
    Unknown(u8),
}

impl Cipher {
    /// O número que vai nas flags.
    pub fn to_bits(self) -> u8 {
        match self {
            Cipher::None => 0,
            Cipher::Salsa2012 => 1,
            Cipher::TrustedPath => 2,
            Cipher::AesGmacSiv => 3,
            Cipher::Unknown(n) => n & 7,
        }
    }

    /// A suíte a partir do número das flags.
    pub fn from_bits(bits: u8) -> Self {
        match bits & 7 {
            0 => Cipher::None,
            1 => Cipher::Salsa2012,
            2 => Cipher::TrustedPath,
            3 => Cipher::AesGmacSiv,
            n => Cipher::Unknown(n),
        }
    }
}

/// Os verbos do VL1 (`Packet::Verb`, `Packet.hpp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Não faz nada.
    Nop,
    /// Apresentação: identidade, versão, hora e endereço visto.
    Hello,
    /// Erro em resposta a um verbo.
    Error,
    /// Sucesso em resposta a um verbo.
    Ok,
    /// Pergunta a identidade de um endereço.
    Whois,
    /// Manda dois nós se contatarem (furo de NAT).
    Rendezvous,
    /// Quadro Ethernet simples (MAC derivado dos endereços).
    Frame,
    /// Quadro Ethernet com MACs explícitos (pontes e regras).
    ExtFrame,
    /// Eco, para medir latência e manter o caminho.
    Echo,
    /// Anuncia a participação num grupo multicast.
    MulticastLike,
    /// Credenciais de rede (COM, capacidades, tags).
    NetworkCredentials,
    /// Pede a configuração de uma rede ao controlador.
    NetworkConfigRequest,
    /// A configuração da rede, em pedaços.
    NetworkConfig,
    /// Pergunta quem está num grupo multicast.
    MulticastGather,
    /// Quadro multicast.
    MulticastFrame,
    /// Caminhos diretos que o outro lado pode tentar.
    PushDirectPaths,
    /// Confirmação (não usado por nós leves).
    Ack,
    /// Medição de qualidade do caminho.
    QosMeasurement,
    /// Mensagem de usuário.
    UserMessage,
    /// Rastreamento remoto.
    RemoteTrace,
    /// Negociação de caminho.
    PathNegotiationRequest,
    /// Um número que o protocolo não define.
    Unknown(u8),
}

impl Verb {
    /// O número no fio (sem o bit de comprimido).
    pub fn to_u8(self) -> u8 {
        match self {
            Verb::Nop => 0x00,
            Verb::Hello => 0x01,
            Verb::Error => 0x02,
            Verb::Ok => 0x03,
            Verb::Whois => 0x04,
            Verb::Rendezvous => 0x05,
            Verb::Frame => 0x06,
            Verb::ExtFrame => 0x07,
            Verb::Echo => 0x08,
            Verb::MulticastLike => 0x09,
            Verb::NetworkCredentials => 0x0a,
            Verb::NetworkConfigRequest => 0x0b,
            Verb::NetworkConfig => 0x0c,
            Verb::MulticastGather => 0x0d,
            Verb::MulticastFrame => 0x0e,
            Verb::PushDirectPaths => 0x10,
            Verb::Ack => 0x12,
            Verb::QosMeasurement => 0x13,
            Verb::UserMessage => 0x14,
            Verb::RemoteTrace => 0x15,
            Verb::PathNegotiationRequest => 0x16,
            Verb::Unknown(n) => n,
        }
    }

    /// O verbo a partir do número (o bit de comprimido é ignorado).
    pub fn from_u8(n: u8) -> Self {
        match n & MASK_VERB {
            0x00 => Verb::Nop,
            0x01 => Verb::Hello,
            0x02 => Verb::Error,
            0x03 => Verb::Ok,
            0x04 => Verb::Whois,
            0x05 => Verb::Rendezvous,
            0x06 => Verb::Frame,
            0x07 => Verb::ExtFrame,
            0x08 => Verb::Echo,
            0x09 => Verb::MulticastLike,
            0x0a => Verb::NetworkCredentials,
            0x0b => Verb::NetworkConfigRequest,
            0x0c => Verb::NetworkConfig,
            0x0d => Verb::MulticastGather,
            0x0e => Verb::MulticastFrame,
            0x10 => Verb::PushDirectPaths,
            0x12 => Verb::Ack,
            0x13 => Verb::QosMeasurement,
            0x14 => Verb::UserMessage,
            0x15 => Verb::RemoteTrace,
            0x16 => Verb::PathNegotiationRequest,
            n => Verb::Unknown(n),
        }
    }
}

/// O código de um ERROR (`Packet::ErrorCode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// Sem erro.
    None,
    /// Pedido malformado.
    InvalidRequest,
    /// Versão de protocolo que o outro lado não aceita.
    BadProtocolVersion,
    /// O objeto pedido não existe (por exemplo, a rede no controlador).
    ObjectNotFound,
    /// Dois nós com o mesmo endereço.
    IdentityCollision,
    /// Operação que o outro lado não suporta.
    UnsupportedOperation,
    /// O outro lado quer o certificado de associação (COM) da rede.
    NeedMembershipCertificate,
    /// O controlador recusou o acesso: o nó não foi autorizado na rede.
    NetworkAccessDenied,
    /// Multicast que o destino não quer.
    UnwantedMulticast,
    /// A rede exige autenticação externa (SSO).
    NetworkAuthenticationRequired,
    /// Um código que o protocolo não define.
    Unknown(u8),
}

impl ErrorCode {
    /// O código a partir do byte do fio.
    pub fn from_u8(n: u8) -> Self {
        match n {
            0 => ErrorCode::None,
            1 => ErrorCode::InvalidRequest,
            2 => ErrorCode::BadProtocolVersion,
            3 => ErrorCode::ObjectNotFound,
            4 => ErrorCode::IdentityCollision,
            5 => ErrorCode::UnsupportedOperation,
            6 => ErrorCode::NeedMembershipCertificate,
            7 => ErrorCode::NetworkAccessDenied,
            8 => ErrorCode::UnwantedMulticast,
            9 => ErrorCode::NetworkAuthenticationRequired,
            n => ErrorCode::Unknown(n),
        }
    }

    /// O byte do fio.
    pub fn to_u8(self) -> u8 {
        match self {
            ErrorCode::None => 0,
            ErrorCode::InvalidRequest => 1,
            ErrorCode::BadProtocolVersion => 2,
            ErrorCode::ObjectNotFound => 3,
            ErrorCode::IdentityCollision => 4,
            ErrorCode::UnsupportedOperation => 5,
            ErrorCode::NeedMembershipCertificate => 6,
            ErrorCode::NetworkAccessDenied => 7,
            ErrorCode::UnwantedMulticast => 8,
            ErrorCode::NetworkAuthenticationRequired => 9,
            ErrorCode::Unknown(n) => n,
        }
    }
}

/// Um pacote do VL1, inteiro, num buffer que cresce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet(Vec<u8>);

impl Packet {
    /// Um pacote novo: cabeçalho com `id`, destino e origem, sem saltos nem cifra, e o verbo.
    /// A carga se acrescenta com [`Packet::append`].
    pub fn new(id: u64, dest: Address, source: Address, verb: Verb) -> Self {
        let mut bytes = Vec::with_capacity(UDP_PAYLOAD_MTU);
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.extend_from_slice(&dest.to_bytes());
        bytes.extend_from_slice(&source.to_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.push(verb.to_u8());
        Self(bytes)
    }

    /// Um pacote vindo do fio. Recusa o que não tem nem o cabeçalho inteiro.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < MIN_PACKET_LEN {
            return Err(Error::Packet(format!(
                "{} bytes, menos que o cabeçalho de {MIN_PACKET_LEN}",
                bytes.len()
            )));
        }
        Ok(Self(bytes))
    }

    /// Os bytes do pacote.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Os bytes do pacote, para a armadura trabalhar no lugar.
    pub fn as_bytes_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }

    /// Devolve os bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// O tamanho total.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Um pacote nunca fica vazio (tem o cabeçalho), mas o clippy pede.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Acrescenta bytes à carga.
    pub fn append(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    /// O ID de 64 bits (a mesma coisa que o IV).
    pub fn id(&self) -> u64 {
        id_of(&self.0)
    }

    /// O endereço do destino.
    pub fn dest(&self) -> Address {
        dest_of(&self.0)
    }

    /// O endereço da origem.
    pub fn source(&self) -> Address {
        Address::from_bytes(&self.0[idx::SOURCE..]).expect("cabeçalho inteiro")
    }

    /// As flags.
    pub fn flags(&self) -> u8 {
        self.0[idx::FLAGS]
    }

    /// Quantos nós intermediários o pacote já passou.
    pub fn hops(&self) -> u8 {
        self.0[idx::FLAGS] & MASK_HOPS
    }

    /// Soma um salto. `false` se já estava no máximo, quando o pacote não deve ser repassado.
    pub fn increment_hops(&mut self) -> bool {
        let saltos = self.hops();
        if saltos >= MAX_HOPS {
            return false;
        }
        self.0[idx::FLAGS] = (self.0[idx::FLAGS] & !MASK_HOPS) | (saltos + 1);
        true
    }

    /// A suíte de cifra declarada.
    pub fn cipher(&self) -> Cipher {
        Cipher::from_bits((self.0[idx::FLAGS] & MASK_CIPHER) >> 3)
    }

    /// Muda a suíte de cifra.
    pub fn set_cipher(&mut self, cifra: Cipher) {
        self.0[idx::FLAGS] = (self.0[idx::FLAGS] & !MASK_CIPHER) | (cifra.to_bits() << 3);
    }

    /// A armadura estendida está ligada.
    pub fn extended_armor(&self) -> bool {
        self.0[idx::FLAGS] & FLAG_EXTENDED_ARMOR != 0
    }

    /// Liga ou desliga a armadura estendida.
    pub fn set_extended_armor(&mut self, ligada: bool) {
        if ligada {
            self.0[idx::FLAGS] |= FLAG_EXTENDED_ARMOR;
        } else {
            self.0[idx::FLAGS] &= !FLAG_EXTENDED_ARMOR;
        }
    }

    /// Vêm fragmentos depois deste pedaço.
    pub fn fragmented(&self) -> bool {
        self.0[idx::FLAGS] & FLAG_FRAGMENTED != 0
    }

    /// Liga ou desliga a marca de fragmentado.
    pub fn set_fragmented(&mut self, fragmentado: bool) {
        if fragmentado {
            self.0[idx::FLAGS] |= FLAG_FRAGMENTED;
        } else {
            self.0[idx::FLAGS] &= !FLAG_FRAGMENTED;
        }
    }

    /// O verbo.
    pub fn verb(&self) -> Verb {
        Verb::from_u8(self.0[idx::VERB])
    }

    /// A carga está comprimida.
    pub fn compressed(&self) -> bool {
        self.0[idx::VERB] & VERB_FLAG_COMPRESSED != 0
    }

    /// A carga do verbo (depois do byte do verbo).
    pub fn payload(&self) -> &[u8] {
        &self.0[idx::PAYLOAD..]
    }

    /// Comprime a carga com LZ4 se isso a diminuir. Não mexe em pacotes pequenos nem nos que já
    /// estão comprimidos. Devolve se comprimiu.
    pub fn compress(&mut self) -> bool {
        // Pacotes de até 64 bytes de carga não valem o custo (`Packet::compress`, `Packet.cpp`).
        const MENOR_CARGA_COMPRIMIDA: usize = 64;
        if self.compressed() || self.payload().len() <= MENOR_CARGA_COMPRIMIDA {
            return false;
        }
        let comprimido = lz4_flex::block::compress(self.payload());
        if comprimido.len() >= self.payload().len() {
            return false;
        }
        self.0.truncate(idx::PAYLOAD);
        self.0.extend_from_slice(&comprimido);
        self.0[idx::VERB] |= VERB_FLAG_COMPRESSED;
        true
    }

    /// Descomprime a carga se estiver comprimida. Falha se o LZ4 for inválido ou se a carga
    /// descomprimida passar do maior pacote que o protocolo prevê.
    pub fn uncompress(&mut self) -> Result<()> {
        if !self.compressed() {
            return Ok(());
        }
        let mut saida = vec![0u8; MAX_PACKET_LEN - idx::PAYLOAD];
        let n = lz4_flex::block::decompress_into(self.payload(), &mut saida)
            .map_err(|e| Error::Packet(format!("LZ4 inválido: {e}")))?;
        self.0.truncate(idx::PAYLOAD);
        self.0.extend_from_slice(&saida[..n]);
        self.0[idx::VERB] &= !VERB_FLAG_COMPRESSED;
        Ok(())
    }
}

/// O ID de um pacote cru ou de um fragmento (os 8 primeiros bytes).
pub fn id_of(bytes: &[u8]) -> u64 {
    let mut id = [0u8; 8];
    id.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(id)
}

/// O destino de um pacote cru ou de um fragmento (bytes 8 a 13).
pub fn dest_of(bytes: &[u8]) -> Address {
    Address::from_bytes(&bytes[idx::DEST..]).expect("cabeçalho inteiro")
}

/// Um fragmento de pacote: tudo depois do primeiro pedaço (`Packet::Fragment`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment(Vec<u8>);

impl Fragment {
    /// Tira o fragmento do fio, se for um: tem o cabeçalho de fragmento e o indicador `0xff`
    /// depois do destino (um pacote normal nunca tem `0xff` no primeiro byte da origem).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FRAGMENT_HEADER_LEN || bytes[fragmento::INDICADOR] != RESERVED_PREFIX {
            return None;
        }
        let f = Self(bytes.to_vec());
        (f.number() >= 1 && f.number() < f.total() && f.total() as usize <= MAX_FRAGMENTS_NO_FIO)
            .then_some(f)
    }

    /// Monta o fragmento `numero` (de 1 em diante) de `total` pedaços, com os bytes `dados`.
    pub fn new(id: u64, dest: Address, numero: u8, total: u8, dados: &[u8]) -> Self {
        let mut bytes = Vec::with_capacity(FRAGMENT_HEADER_LEN + dados.len());
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.extend_from_slice(&dest.to_bytes());
        bytes.push(RESERVED_PREFIX);
        bytes.push((total << 4) | (numero & 0x0f));
        bytes.push(0);
        bytes.extend_from_slice(dados);
        Self(bytes)
    }

    /// Os bytes do fragmento.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// O ID do pacote a que pertence.
    pub fn packet_id(&self) -> u64 {
        id_of(&self.0)
    }

    /// O destino.
    pub fn dest(&self) -> Address {
        dest_of(&self.0)
    }

    /// Quantos pedaços tem o pacote, contando o primeiro.
    pub fn total(&self) -> u8 {
        self.0[fragmento::NUMERO] >> 4
    }

    /// O número deste pedaço (o primeiro, que é o pacote cortado, é o 0).
    pub fn number(&self) -> u8 {
        self.0[fragmento::NUMERO] & 0x0f
    }

    /// Saltos do fragmento.
    pub fn hops(&self) -> u8 {
        self.0[fragmento::SALTOS] & MASK_HOPS
    }

    /// Soma um salto. `false` se já estava no máximo.
    pub fn increment_hops(&mut self) -> bool {
        let saltos = self.hops();
        if saltos >= MAX_HOPS {
            return false;
        }
        self.0[fragmento::SALTOS] = (self.0[fragmento::SALTOS] & !MASK_HOPS) | (saltos + 1);
        true
    }

    /// Os dados do pedaço.
    pub fn data(&self) -> &[u8] {
        &self.0[FRAGMENT_HEADER_LEN..]
    }
}

/// Divide um pacote armado em `[primeiro pedaço, fragmentos...]` quando passa do MTU. O
/// primeiro pedaço leva a marca de fragmentado e o cabeçalho inteiro; os demais, o cabeçalho
/// de fragmento. `Err` se o pacote pedir mais pedaços do que o protocolo permite.
pub fn split(pacote: &[u8]) -> Result<Vec<Vec<u8>>> {
    if pacote.len() <= UDP_PAYLOAD_MTU {
        return Ok(vec![pacote.to_vec()]);
    }
    let dados_por_fragmento = UDP_PAYLOAD_MTU - FRAGMENT_HEADER_LEN;
    let resto = pacote.len() - UDP_PAYLOAD_MTU;
    let fragmentos = resto.div_ceil(dados_por_fragmento);
    let total = fragmentos + 1;
    if total > MAX_FRAGMENTS {
        return Err(Error::Packet(format!(
            "o pacote de {} bytes pediria {total} pedaços (máximo {MAX_FRAGMENTS})",
            pacote.len()
        )));
    }
    let mut pedacos = Vec::with_capacity(total);
    let mut primeiro = pacote[..UDP_PAYLOAD_MTU].to_vec();
    primeiro[idx::FLAGS] |= FLAG_FRAGMENTED;
    pedacos.push(primeiro);
    let id = id_of(pacote);
    let dest = dest_of(pacote);
    for (i, dados) in pacote[UDP_PAYLOAD_MTU..]
        .chunks(dados_por_fragmento)
        .enumerate()
    {
        let f = Fragment::new(id, dest, (i + 1) as u8, total as u8, dados);
        pedacos.push(f.0);
    }
    Ok(pedacos)
}

/// Junta os pedaços de um pacote na ordem dos números: o primeiro e os fragmentos 1 em diante
/// (`fragmentos[i]` é o fragmento `i + 1`). A marca de fragmentado **fica** nas flags: ela entra
/// na mistura da chave do MAC (`Packet::armor`, `Packet.cpp`), então o pacote juntado precisa
/// dela para ser conferido, e quem o armou a ligou antes de armar.
pub fn join(primeiro: &[u8], fragmentos: &[Fragment]) -> Vec<u8> {
    let mut pacote = primeiro.to_vec();
    for f in fragmentos {
        pacote.extend_from_slice(f.data());
    }
    pacote
}

#[cfg(test)]
mod testes {
    use super::*;

    fn exemplo(tamanho: usize) -> Packet {
        let mut p = Packet::new(
            0x0102_0304_0506_0708,
            Address::new(0x11_2233_4455),
            Address::new(0x66_7788_99aa),
            Verb::Echo,
        );
        p.append(&vec![b'a'; tamanho]);
        p
    }

    #[test]
    fn o_cabecalho_tem_os_campos_nos_lugares() {
        let p = exemplo(4);
        assert_eq!(p.id(), 0x0102_0304_0506_0708);
        assert_eq!(p.dest().to_string(), "1122334455");
        assert_eq!(p.source().to_string(), "66778899aa");
        assert_eq!(p.verb(), Verb::Echo);
        assert_eq!(p.payload(), b"aaaa");
        assert_eq!(p.len(), MIN_PACKET_LEN + 4);
    }

    #[test]
    fn as_flags_nao_se_misturam() {
        let mut p = exemplo(0);
        p.set_cipher(Cipher::Salsa2012);
        p.set_extended_armor(true);
        p.set_fragmented(true);
        assert!(p.increment_hops());
        assert_eq!(p.flags(), 0x80 | 0x40 | (1 << 3) | 1);
        assert_eq!(p.cipher(), Cipher::Salsa2012);
        assert_eq!(p.hops(), 1);
        p.set_cipher(Cipher::AesGmacSiv);
        assert_eq!(p.cipher(), Cipher::AesGmacSiv);
        assert_eq!(p.hops(), 1);
        for _ in 0..10 {
            p.increment_hops();
        }
        assert_eq!(p.hops(), MAX_HOPS);
        assert!(!p.increment_hops());
    }

    #[test]
    fn lz4_vai_e_volta_e_so_comprime_o_que_compensa() {
        let mut p = exemplo(2000);
        let original = p.clone();
        assert!(p.compress());
        assert!(p.compressed());
        assert!(p.len() < original.len());
        p.uncompress().unwrap();
        assert_eq!(p, original);

        let mut pequeno = exemplo(10);
        assert!(!pequeno.compress());
        let mut aleatorio = Packet::new(1, Address::new(1), Address::new(2), Verb::Frame);
        let mut bytes = [0u8; 200];
        getrandom::fill(&mut bytes).unwrap();
        aleatorio.append(&bytes);
        assert!(!aleatorio.compress(), "dado aleatório não comprime");
    }

    #[test]
    fn lz4_invalido_ou_grande_demais_e_recusado() {
        let mut p = exemplo(100);
        p.0[idx::VERB] |= VERB_FLAG_COMPRESSED;
        p.0.truncate(idx::PAYLOAD);
        p.append(&[0xff, 0xff, 0xff, 0xff, 0xff]);
        assert!(p.uncompress().is_err());

        // Uma bomba: 1 MiB de zeros comprimido não cabe no maior pacote do protocolo.
        let bomba = lz4_flex::block::compress(&vec![0u8; 1 << 20]);
        let mut p = exemplo(0);
        p.0[idx::VERB] |= VERB_FLAG_COMPRESSED;
        p.append(&bomba);
        assert!(p.uncompress().is_err());
    }

    #[test]
    fn divide_e_junta_um_pacote_grande() {
        let mut p = exemplo(0);
        let mut dados = vec![0u8; 3000];
        getrandom::fill(&mut dados).unwrap();
        p.append(&dados);
        let pedacos = split(p.as_bytes()).unwrap();
        assert_eq!(pedacos.len(), 3);
        assert_eq!(pedacos[0].len(), UDP_PAYLOAD_MTU);
        assert!(pedacos[0][idx::FLAGS] & FLAG_FRAGMENTED != 0);
        assert!(
            Fragment::parse(&pedacos[0]).is_none(),
            "o primeiro é um pacote"
        );
        let fragmentos: Vec<Fragment> = pedacos[1..]
            .iter()
            .map(|b| Fragment::parse(b).expect("fragmento"))
            .collect();
        assert_eq!(fragmentos[0].number(), 1);
        assert_eq!(fragmentos[1].number(), 2);
        assert_eq!(fragmentos[0].total(), 3);
        assert_eq!(fragmentos[0].packet_id(), p.id());
        assert_eq!(fragmentos[0].dest(), p.dest());
        let mut esperado = p.clone();
        esperado.set_fragmented(true);
        assert_eq!(join(&pedacos[0], &fragmentos), esperado.as_bytes());
    }

    #[test]
    fn pacote_pequeno_nao_se_divide_e_gigante_e_recusado() {
        assert_eq!(split(exemplo(100).as_bytes()).unwrap().len(), 1);
        assert!(split(exemplo(MAX_PACKET_LEN).as_bytes()).is_err());
    }

    #[test]
    fn pacote_curto_e_recusado() {
        assert!(Packet::from_bytes(vec![0; MIN_PACKET_LEN - 1]).is_err());
        assert!(Packet::from_bytes(vec![0; MIN_PACKET_LEN]).is_ok());
    }
}
