//! Os temporizadores do motor: uma fila ordenada pelo instante, sem uma tarefa por temporizador.
//!
//! O cancelamento é preguiçoso: quem agendou guarda o instante esperado e ignora o disparo que
//! não confere com ele (o motor faz isso nos apertos de mão e nos túneis).

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use tokio::time::Instant;

struct Entry<K> {
    when: Instant,
    seq: u64,
    key: K,
}

impl<K> PartialEq for Entry<K> {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when && self.seq == other.seq
    }
}

impl<K> Eq for Entry<K> {}

impl<K> PartialOrd for Entry<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for Entry<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Empata pela ordem em que foram agendados.
        (self.when, self.seq).cmp(&(other.when, other.seq))
    }
}

/// A fila de temporizadores.
pub(crate) struct Timers<K> {
    heap: BinaryHeap<Reverse<Entry<K>>>,
    seq: u64,
}

impl<K> Timers<K> {
    pub(crate) fn new() -> Timers<K> {
        Timers {
            heap: BinaryHeap::new(),
            seq: 0,
        }
    }

    /// Agenda `key` para `when`.
    pub(crate) fn add(&mut self, when: Instant, key: K) {
        self.seq += 1;
        self.heap.push(Reverse(Entry {
            when,
            seq: self.seq,
            key,
        }));
    }

    /// Quando o próximo temporizador vence.
    pub(crate) fn next(&self) -> Option<Instant> {
        self.heap.peek().map(|Reverse(e)| e.when)
    }

    /// Tira o próximo que já venceu em `now`, junto com o instante em que foi agendado.
    pub(crate) fn pop_due(&mut self, now: Instant) -> Option<(Instant, K)> {
        if self.heap.peek().is_some_and(|Reverse(e)| e.when <= now) {
            self.heap.pop().map(|Reverse(e)| (e.when, e.key))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn dispara_na_ordem_do_instante_e_empata_pela_ordem_de_agendamento() {
        let base = Instant::now();
        let mut t = Timers::new();
        t.add(base + Duration::from_secs(2), "c");
        t.add(base + Duration::from_secs(1), "a");
        t.add(base + Duration::from_secs(1), "b");
        assert_eq!(t.next(), Some(base + Duration::from_secs(1)));
        assert!(t.pop_due(base).is_none());
        let fim = base + Duration::from_secs(5);
        let ordem: Vec<_> = std::iter::from_fn(|| t.pop_due(fim).map(|(_, k)| k)).collect();
        assert_eq!(ordem, ["a", "b", "c"]);
        assert!(t.next().is_none());
    }
}
