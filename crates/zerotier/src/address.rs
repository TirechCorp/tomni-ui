//! Endereços do ZeroTier: o do nó (40 bits) e o da rede virtual (64 bits).

use std::fmt;

/// O endereço de um nó: 40 bits, escrito em 10 dígitos hexadecimais (`8056c2e21c`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Address(u64);

/// Tamanho do endereço no fio, em bytes (protocolo do ZeroTier: `ZT_ADDRESS_LENGTH`, `Constants.hpp`).
pub const ADDRESS_LEN: usize = 5;

/// Primeiro byte que o protocolo reserva: endereços que começam com ele nunca são de um nó e o
/// byte marca os fragmentos de pacote (`ZT_ADDRESS_RESERVED_PREFIX`, `Constants.hpp`).
pub const RESERVED_PREFIX: u8 = 0xff;

impl Address {
    /// O endereço a partir dos 40 bits menos significativos de `valor`.
    pub fn new(valor: u64) -> Self {
        Self(valor & 0xff_ffff_ffff)
    }

    /// O endereço a partir dos 5 bytes do fio (ordem de rede).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes: &[u8; ADDRESS_LEN] = bytes.get(..ADDRESS_LEN)?.try_into().ok()?;
        let mut inteiro = [0u8; 8];
        inteiro[8 - ADDRESS_LEN..].copy_from_slice(bytes);
        Some(Self(u64::from_be_bytes(inteiro)))
    }

    /// Lê os 10 dígitos hexadecimais; `None` se não forem exatamente 10.
    pub fn parse(texto: &str) -> Option<Self> {
        if texto.len() != ADDRESS_LEN * 2 || !texto.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u64::from_str_radix(texto, 16).ok().map(Self)
    }

    /// Os 5 bytes do fio.
    pub fn to_bytes(self) -> [u8; ADDRESS_LEN] {
        let inteiro = self.0.to_be_bytes();
        let mut saida = [0u8; ADDRESS_LEN];
        saida.copy_from_slice(&inteiro[8 - ADDRESS_LEN..]);
        saida
    }

    /// O valor de 40 bits.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Endereço reservado: zero ou começando em `0xff`. Nenhum nó tem um.
    pub fn is_reserved(self) -> bool {
        self.0 == 0 || (self.0 >> 32) as u8 == RESERVED_PREFIX
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:010x}", self.0)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({self})")
    }
}

/// O identificador de uma rede virtual: 64 bits, 16 dígitos hexadecimais. Os 40 bits de cima são
/// o endereço do controlador da rede (`8056c2e21c000001`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct NetworkId(u64);

impl NetworkId {
    /// O identificador a partir do inteiro de 64 bits.
    pub fn new(valor: u64) -> Self {
        Self(valor)
    }

    /// Lê os 16 dígitos hexadecimais; `None` se não forem exatamente 16.
    pub fn parse(texto: &str) -> Option<Self> {
        if texto.len() != 16 || !texto.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u64::from_str_radix(texto, 16).ok().map(Self)
    }

    /// O valor de 64 bits.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// O endereço do controlador: os 40 bits de cima.
    pub fn controller(self) -> Address {
        Address::new(self.0 >> 24)
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NetworkId({self})")
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn endereco_vai_e_volta_pelo_texto_e_pelo_fio() {
        let a = Address::parse("8056c2e21c").unwrap();
        assert_eq!(a.to_string(), "8056c2e21c");
        assert_eq!(a.to_bytes(), [0x80, 0x56, 0xc2, 0xe2, 0x1c]);
        assert_eq!(Address::from_bytes(&a.to_bytes()), Some(a));
        assert!(Address::parse("8056c2e21").is_none());
        assert!(Address::parse("8056c2e21z").is_none());
    }

    #[test]
    fn reservados_sao_zero_e_ff() {
        assert!(Address::new(0).is_reserved());
        assert!(Address::new(0xff_0000_0001).is_reserved());
        assert!(!Address::new(0xfe_0000_0001).is_reserved());
    }

    #[test]
    fn a_rede_diz_quem_e_o_controlador() {
        let rede = NetworkId::parse("8056c2e21c000001").unwrap();
        assert_eq!(rede.controller().to_string(), "8056c2e21c");
        assert_eq!(rede.to_string(), "8056c2e21c000001");
        assert!(NetworkId::parse("8056c2e21c0000").is_none());
    }
}
