//! O registro dos índices de receptor.
//!
//! Cada handshake e cada chave de sessão têm um índice de 32 bits sorteado, que o outro lado
//! põe nos pacotes que manda para nós. O registro diz de qual par é cada índice, para o
//! dispositivo entregar o pacote ao par certo sem tentar abri-lo em todos.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::key::{KeyError, PublicKey, random_bytes};

/// O registro, compartilhado entre o dispositivo e os pares.
#[derive(Clone, Default)]
pub struct Indices {
    mapa: Arc<Mutex<HashMap<u32, PublicKey>>>,
}

impl Indices {
    /// Um registro vazio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sorteia um índice livre para o par. O índice volta ao registro quando o
    /// [`IndexGuard`] é solto.
    pub fn allocate(&self, par: PublicKey) -> Result<IndexGuard, KeyError> {
        let mut mapa = self.mapa.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let id = u32::from_le_bytes(random_bytes::<4>()?);
            if let std::collections::hash_map::Entry::Vacant(vaga) = mapa.entry(id) {
                vaga.insert(par);
                return Ok(IndexGuard {
                    id,
                    registro: self.clone(),
                });
            }
        }
    }

    /// De qual par é o índice, se estiver em uso.
    pub fn lookup(&self, id: u32) -> Option<PublicKey> {
        self.mapa
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .copied()
    }

    /// Quantos índices estão em uso.
    pub fn len(&self) -> usize {
        self.mapa
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Se não há índice em uso.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Um índice reservado; ao ser solto, devolve o índice ao registro.
pub struct IndexGuard {
    id: u32,
    registro: Indices,
}

impl IndexGuard {
    /// O valor do índice.
    pub fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for IndexGuard {
    fn drop(&mut self) {
        self.registro
            .mapa
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl std::fmt::Debug for IndexGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IndexGuard({})", self.id)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_indice_e_unico_e_volta_ao_soltar() {
        let reg = Indices::new();
        let par = PublicKey::from_bytes([1; 32]);
        let a = reg.allocate(par).unwrap();
        let b = reg.allocate(par).unwrap();
        assert_ne!(a.id(), b.id());
        assert_eq!(reg.lookup(a.id()), Some(par));
        assert_eq!(reg.len(), 2);
        let id_a = a.id();
        drop(a);
        assert_eq!(reg.lookup(id_a), None);
        assert_eq!(reg.len(), 1);
        drop(b);
        assert!(reg.is_empty());
    }
}
