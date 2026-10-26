//! O filtro de saída: a "firewall só de saída". Antes de um pacote que chega ir para a pilha,
//! o filtro descarta, em silêncio, tudo o que não pertence a uma conexão aberta pelo app.
//!
//! Assim a pilha não responde RST, não responde ping e não tem porta aberta: para a rede, o app
//! é invisível. Isso também impede que o `smoltcp` responda sozinho (ele devolve RST a um SYN sem
//! socket e eco a um pedido de ICMP).

use std::collections::HashSet;
use std::net::Ipv4Addr;

/// O número do protocolo TCP no cabeçalho IPv4 (RFC 791).
const PROTO_TCP: u8 = 6;
/// O bit SYN do TCP (RFC 9293).
const SYN: u8 = 0x02;
/// O bit ACK do TCP (RFC 9293).
const ACK: u8 = 0x10;

/// A identidade de uma conexão aberta pelo app, vista pelo lado de quem responde: a porta local e
/// o endereço e a porta do servidor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Flow {
    pub(crate) local_port: u16,
    pub(crate) remote: Ipv4Addr,
    pub(crate) remote_port: u16,
}

/// Quais pacotes de entrada passam.
#[derive(Default)]
pub(crate) struct Filter {
    flows: HashSet<Flow>,
    /// As portas em escuta (só no `test-util`: o app não escuta nada).
    listeners: HashSet<u16>,
    /// Aceita pacotes para qualquer destino (só no `test-util`: o gateway dos testes).
    #[cfg(feature = "test-util")]
    any_destination: bool,
}

impl Filter {
    pub(crate) fn allow(&mut self, flow: Flow) {
        self.flows.insert(flow);
    }

    pub(crate) fn forget(&mut self, flow: &Flow) {
        self.flows.remove(flow);
    }

    #[cfg(feature = "test-util")]
    pub(crate) fn set_any_destination(&mut self, on: bool) {
        self.any_destination = on;
    }

    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn listen(&mut self, port: u16) {
        self.listeners.insert(port);
    }

    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn unlisten(&mut self, port: u16) {
        self.listeners.remove(&port);
    }

    /// `true` se o pacote IP que chega pode ir para a pilha. `locals` são os endereços da pilha.
    pub(crate) fn admit(&self, packet: &[u8], locals: &[Ipv4Addr]) -> bool {
        // Só IPv4 com o cabeçalho completo; o IPv6 ainda não é suportado (M48, adiável).
        let Some(&first) = packet.first() else {
            return false;
        };
        if first >> 4 != 4 {
            return false;
        }
        let header_len = usize::from(first & 0x0F) * 4;
        if header_len < 20 || packet.len() < header_len + 20 {
            return false;
        }
        // Fragmentos (MF ou deslocamento) não são remontados: descartados.
        let fragment = u16::from_be_bytes([packet[6], packet[7]]);
        if fragment & 0x3FFF != 0 {
            return false;
        }
        if packet[9] != PROTO_TCP {
            return false;
        }
        let destination = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
        #[cfg(feature = "test-util")]
        let qualquer = self.any_destination;
        #[cfg(not(feature = "test-util"))]
        let qualquer = false;
        if !qualquer && !locals.contains(&destination) {
            return false;
        }
        let source = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
        let tcp = &packet[header_len..];
        let source_port = u16::from_be_bytes([tcp[0], tcp[1]]);
        let destination_port = u16::from_be_bytes([tcp[2], tcp[3]]);
        let flags = tcp[13];
        if self.listeners.contains(&destination_port) {
            return true;
        }
        // Um SYN sem ACK é um pedido de conexão nova: só quem escuta o aceita.
        if flags & SYN != 0 && flags & ACK == 0 {
            return false;
        }
        self.flows.contains(&Flow {
            local_port: destination_port,
            remote: source,
            remote_port: source_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um pacote TCP mínimo (sem somas de verificação; o filtro não as confere).
    fn tcp(src: [u8; 4], sport: u16, dst: [u8; 4], dport: u16, flags: u8) -> Vec<u8> {
        let mut p = vec![0u8; 40];
        p[0] = 0x45;
        p[9] = PROTO_TCP;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p[20..22].copy_from_slice(&sport.to_be_bytes());
        p[22..24].copy_from_slice(&dport.to_be_bytes());
        p[33] = flags;
        p
    }

    const EU: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const ELE: [u8; 4] = [10, 0, 0, 2];

    #[test]
    fn so_a_resposta_de_uma_conexao_aberta_passa() {
        let mut filtro = Filter::default();
        filtro.allow(Flow {
            local_port: 50000,
            remote: Ipv4Addr::from(ELE),
            remote_port: 22,
        });
        let locais = [EU];
        assert!(filtro.admit(&tcp(ELE, 22, [10, 0, 0, 1], 50000, SYN | ACK), &locais));
        assert!(filtro.admit(&tcp(ELE, 22, [10, 0, 0, 1], 50000, ACK), &locais));
        // Outra porta de origem, outra porta local e outro servidor: descartados.
        assert!(!filtro.admit(&tcp(ELE, 23, [10, 0, 0, 1], 50000, ACK), &locais));
        assert!(!filtro.admit(&tcp(ELE, 22, [10, 0, 0, 1], 50001, ACK), &locais));
        assert!(!filtro.admit(&tcp([10, 0, 0, 9], 22, [10, 0, 0, 1], 50000, ACK), &locais));
        // Um SYN puro nunca é resposta.
        assert!(!filtro.admit(&tcp(ELE, 22, [10, 0, 0, 1], 50000, SYN), &locais));
    }

    #[test]
    fn descarta_o_que_nao_e_tcp_nem_ipv4_nem_para_nos() {
        let mut filtro = Filter::default();
        filtro.allow(Flow {
            local_port: 50000,
            remote: Ipv4Addr::from(ELE),
            remote_port: 22,
        });
        let locais = [EU];
        let mut udp = tcp(ELE, 22, [10, 0, 0, 1], 50000, ACK);
        udp[9] = 17;
        assert!(!filtro.admit(&udp, &locais));
        let mut icmp = tcp(ELE, 22, [10, 0, 0, 1], 50000, ACK);
        icmp[9] = 1;
        assert!(!filtro.admit(&icmp, &locais));
        let mut v6 = tcp(ELE, 22, [10, 0, 0, 1], 50000, ACK);
        v6[0] = 0x60;
        assert!(!filtro.admit(&v6, &locais));
        assert!(!filtro.admit(&tcp(ELE, 22, [10, 0, 0, 7], 50000, ACK), &locais));
        let mut fragmento = tcp(ELE, 22, [10, 0, 0, 1], 50000, ACK);
        fragmento[6] = 0x20;
        assert!(!filtro.admit(&fragmento, &locais));
        assert!(!filtro.admit(&[], &locais));
        assert!(!filtro.admit(&[0x45; 10], &locais));
    }

    #[test]
    fn a_porta_em_escuta_aceita_o_syn() {
        let mut filtro = Filter::default();
        filtro.listen(7);
        assert!(filtro.admit(&tcp(ELE, 40000, [10, 0, 0, 1], 7, SYN), &[EU]));
        assert!(!filtro.admit(&tcp(ELE, 40000, [10, 0, 0, 1], 8, SYN), &[EU]));
        filtro.unlisten(7);
        assert!(!filtro.admit(&tcp(ELE, 40000, [10, 0, 0, 1], 7, SYN), &[EU]));
    }
}
