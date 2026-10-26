//! As regras da rede (`ZT_VirtualNetworkRule`, `ZeroTierOne.h`; avaliação em `_doZtFilter`,
//! `Network.cpp`). Cada regra é `tipo (1) ‖ tamanho do campo (1) ‖ campo`. O bit 0x80 do tipo
//! nega o casamento, o 0x40 troca o E pelo OU, e os 6 bits baixos dizem o que a regra é.
//!
//! Conjuntos de casamentos se combinam por E (ou OU) e terminam numa ação; a ação só vale se o
//! conjunto casou. Sem nenhuma ação que aceite, o quadro é descartado.

use crate::Address;

/// O máximo de regras de uma rede (`ZT_MAX_NETWORK_RULES`).
pub const MAX_RULES: usize = 1024;

const NOT: u8 = 0x80;
const OR: u8 = 0x40;
const TYPE_MASK: u8 = 0x3f;

const ACTION_DROP: u8 = 0;
const ACTION_ACCEPT: u8 = 1;
const ACTION_TEE: u8 = 2;
const ACTION_WATCH: u8 = 3;
const ACTION_REDIRECT: u8 = 4;
const ACTION_BREAK: u8 = 5;
const ACTION_PRIORITY: u8 = 6;
/// A última ação (`ZT_NETWORK_RULE_ACTION__MAX_ID`).
const ACTION_MAX: u8 = 15;

const M_SRC_ZT: u8 = 24;
const M_DEST_ZT: u8 = 25;
const M_VLAN_ID: u8 = 26;
const M_VLAN_PCP: u8 = 27;
const M_VLAN_DEI: u8 = 28;
const M_MAC_SRC: u8 = 29;
const M_MAC_DEST: u8 = 30;
const M_IPV4_SRC: u8 = 31;
const M_IPV4_DEST: u8 = 32;
const M_IPV6_SRC: u8 = 33;
const M_IPV6_DEST: u8 = 34;
const M_IP_TOS: u8 = 35;
const M_IP_PROTOCOL: u8 = 36;
const M_ETHERTYPE: u8 = 37;
const M_ICMP: u8 = 38;
const M_SRC_PORT: u8 = 39;
const M_DEST_PORT: u8 = 40;
const M_CHARACTERISTICS: u8 = 41;
const M_FRAME_SIZE: u8 = 42;
const M_RANDOM: u8 = 43;
const M_TAGS_DIFFERENCE: u8 = 44;
const M_TAGS_AND: u8 = 45;
const M_TAGS_OR: u8 = 46;
const M_TAGS_XOR: u8 = 47;
const M_TAGS_EQUAL: u8 = 48;
const M_TAG_SENDER: u8 = 49;
const M_TAG_RECEIVER: u8 = 50;
const M_INTEGER_RANGE: u8 = 51;

/// Característica: o quadro está entrando (`ZT_RULE_PACKET_CHARACTERISTICS_INBOUND`).
const CHAR_INBOUND: u64 = 0x8000_0000_0000_0000;
/// Característica: o destino é multicast.
const CHAR_MULTICAST: u64 = 0x4000_0000_0000_0000;
/// Característica: o destino é broadcast.
const CHAR_BROADCAST: u64 = 0x2000_0000_0000_0000;
/// Característica: o MAC de origem foi autenticado pelo endereço de quem mandou. A rede que usa
/// certificado de propriedade (adiável) acrescentaria também a do IP.
const CHAR_SENDER_MAC_AUTHENTICATED: u64 = 0x0800_0000_0000_0000;

/// A flag da config (`ZT_NETWORKCONFIG_FLAG_RULES_RESULT_OF_UNSUPPORTED_MATCH`): o que um
/// casamento que não conhecemos devolve.
pub const FLAG_UNSUPPORTED_MATCH: u64 = 0x08;

const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV6: u16 = 0x86dd;

/// Uma regra, como veio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// O byte do tipo, com os bits de negação e de OU.
    pub t: u8,
    /// O campo, no formato do fio.
    pub field: Vec<u8>,
}

impl Rule {
    /// Uma regra sem campo (as ações DROP, ACCEPT e BREAK).
    pub fn action(t: u8) -> Self {
        Self { t, field: vec![] }
    }

    /// DROP.
    pub fn drop() -> Self {
        Self::action(ACTION_DROP)
    }

    /// ACCEPT.
    pub fn accept() -> Self {
        Self::action(ACTION_ACCEPT)
    }

    /// Casa com este tipo de quadro (`ethertype`), com a negação opcional.
    pub fn ethertype(tipo: u16, nao: bool) -> Self {
        Self {
            t: M_ETHERTYPE | if nao { NOT } else { 0 },
            field: tipo.to_be_bytes().to_vec(),
        }
    }

    /// Casa com o protocolo IP.
    pub fn ip_protocol(proto: u8, nao: bool) -> Self {
        Self {
            t: M_IP_PROTOCOL | if nao { NOT } else { 0 },
            field: vec![proto],
        }
    }

    /// Casa com a porta de destino entre `de` e `ate`.
    pub fn dest_port(de: u16, ate: u16, nao: bool) -> Self {
        let mut campo = de.to_be_bytes().to_vec();
        campo.extend_from_slice(&ate.to_be_bytes());
        Self {
            t: M_DEST_PORT | if nao { NOT } else { 0 },
            field: campo,
        }
    }

    /// TEE (cópia) para `para`.
    pub fn tee(para: Address, tamanho: u16) -> Self {
        Self::forward(ACTION_TEE, para, tamanho)
    }

    /// REDIRECT para `para`.
    pub fn redirect(para: Address) -> Self {
        Self::forward(ACTION_REDIRECT, para, 0)
    }

    fn forward(acao: u8, para: Address, tamanho: u16) -> Self {
        let mut campo = para.as_u64().to_be_bytes().to_vec();
        campo.extend_from_slice(&0u32.to_be_bytes());
        campo.extend_from_slice(&tamanho.to_be_bytes());
        Self {
            t: acao,
            field: campo,
        }
    }

    fn kind(&self) -> u8 {
        self.t & TYPE_MASK
    }
}

/// Escreve as regras no formato do fio.
pub fn serialize(regras: &[Rule]) -> Vec<u8> {
    let mut saida = Vec::new();
    for r in regras {
        saida.push(r.t);
        saida.push(u8::try_from(r.field.len()).unwrap_or(u8::MAX));
        saida.extend_from_slice(&r.field);
    }
    saida
}

/// Lê as regras. Um tipo que não conhecemos é guardado do mesmo jeito (o tamanho do campo diz
/// quanto pular). Devolve `None` se o fim cai no meio de uma regra.
pub fn parse(bytes: &[u8]) -> Option<Vec<Rule>> {
    let mut regras = Vec::new();
    let mut p = 0;
    while p < bytes.len() && regras.len() < MAX_RULES {
        let t = *bytes.get(p)?;
        let tamanho = usize::from(*bytes.get(p + 1)?);
        p += 2;
        let campo = bytes.get(p..p + tamanho)?;
        p += tamanho;
        regras.push(Rule {
            t,
            field: campo.to_vec(),
        });
    }
    Some(regras)
}

/// O que se sabe de um quadro na hora de avaliar.
pub struct FrameView<'a> {
    /// O endereço de quem manda.
    pub zt_source: Address,
    /// O endereço de quem recebe.
    pub zt_dest: Address,
    /// O MAC de origem.
    pub mac_source: [u8; 6],
    /// O MAC de destino.
    pub mac_dest: [u8; 6],
    /// O tipo do quadro.
    pub ethertype: u16,
    /// A carga, depois do cabeçalho Ethernet.
    pub data: &'a [u8],
    /// O quadro está entrando.
    pub inbound: bool,
    /// O MAC de origem deriva do endereço de quem mandou.
    pub mac_authenticated: bool,
}

/// A conta do resultado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Deixa passar.
    Accept,
    /// Descarta.
    Drop,
    /// Manda a outro endereço em vez do destino.
    Redirect(Address),
}

/// A resposta completa da avaliação.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// O veredito.
    pub verdict: Verdict,
    /// Se há cópia a mandar (TEE ou WATCH): para onde, quantos bytes do quadro e se é WATCH.
    pub copy: Option<(Address, usize, bool)>,
}

/// Avalia `regras` para `quadro`. `nosso` é o nosso endereço; `flags` são as flags da config;
/// `sorteio` devolve um `u32` aleatório (o casamento RANDOM).
///
/// As tags que a rede define (e as capabilities) não são tratadas: o app não tem tags, então
/// um casamento de tags com tag que o nosso lado não tem dá "não casou", e os de tag do outro
/// lado dão "não casou" na entrada e "casou, sem descartar" na saída (como o ZeroTier faz
/// enquanto não sabe as tags do destinatário).
pub fn evaluate(
    regras: &[Rule],
    quadro: &FrameView<'_>,
    nosso: Address,
    flags: u64,
    mut sorteio: impl FnMut() -> u32,
) -> Outcome {
    let mut super_accept = false;
    let mut este_conjunto = true;
    let mut pular_drop = false;
    let mut copia: Option<(Address, usize, bool)> = None;
    let sem_aceite = |copia| Outcome {
        verdict: Verdict::Drop,
        copy: copia,
    };

    for regra in regras {
        let tipo = regra.kind();
        if tipo <= ACTION_MAX {
            if !este_conjunto {
                if quadro.inbound
                    && matches!(tipo, ACTION_TEE | ACTION_WATCH | ACTION_REDIRECT)
                    && fwd_address(regra) == Some(nosso)
                {
                    super_accept = true;
                }
                este_conjunto = true;
                continue;
            }
            match tipo {
                ACTION_PRIORITY | ACTION_ACCEPT => {
                    return Outcome {
                        verdict: Verdict::Accept,
                        copy: copia,
                    };
                }
                ACTION_DROP => {
                    if pular_drop {
                        pular_drop = false;
                        continue;
                    }
                    return sem_aceite(copia);
                }
                ACTION_TEE | ACTION_WATCH | ACTION_REDIRECT => {
                    let Some(alvo) = fwd_address(regra) else {
                        continue;
                    };
                    if alvo == quadro.zt_source {
                        // Sem efeito: a origem já é o alvo.
                    } else if alvo == nosso {
                        if quadro.inbound {
                            return Outcome {
                                verdict: Verdict::Accept,
                                copy: copia,
                            };
                        }
                    } else if alvo == quadro.zt_dest {
                        // Sem efeito: o destino já é o alvo.
                    } else if tipo == ACTION_REDIRECT {
                        return Outcome {
                            verdict: Verdict::Redirect(alvo),
                            copy: copia,
                        };
                    } else {
                        let pedido = regra
                            .field
                            .get(12..14)
                            .map_or(0, |b| usize::from(u16::from_be_bytes([b[0], b[1]])));
                        let n = if pedido != 0 {
                            pedido.min(quadro.data.len() + 14)
                        } else {
                            quadro.data.len() + 14
                        };
                        copia = Some((alvo, n, tipo == ACTION_WATCH));
                    }
                    continue;
                }
                ACTION_BREAK => return sem_aceite(copia),
                _ => continue,
            }
        }

        // Disjunção de curto-circuito: E com falso é falso.
        if !este_conjunto && regra.t & OR == 0 {
            continue;
        }
        let nao = regra.t & NOT != 0;
        let (casou, pula) = casar(regra, quadro, nao, super_accept, flags, &mut sorteio);
        if pula {
            pular_drop = true;
        }
        if regra.t & OR != 0 {
            este_conjunto |= casou ^ nao;
        } else {
            este_conjunto &= casou ^ nao;
        }
    }
    // Nenhuma ação aceitou: o chamador trata como descarte.
    sem_aceite(copia)
}

fn fwd_address(regra: &Rule) -> Option<Address> {
    let b = regra.field.get(..8)?;
    Some(Address::new(u64::from_be_bytes(b.try_into().ok()?)))
}

/// Avalia um casamento. Devolve se casou (antes da negação do conjunto, que quem chama aplica)
/// e se o próximo DROP deve ser pulado.
fn casar(
    regra: &Rule,
    q: &FrameView<'_>,
    nao: bool,
    super_accept: bool,
    flags: u64,
    sorteio: &mut impl FnMut() -> u32,
) -> (bool, bool) {
    let hard_no = nao;
    let hard_yes = !nao;
    let f = &regra.field;
    let d = q.data;
    let ipv4 = q.ethertype == ETHERTYPE_IPV4 && d.len() >= 20;
    let ipv6 = q.ethertype == ETHERTYPE_IPV6;
    let campo_curto = f.len() < min_field(regra.kind());
    if campo_curto {
        // Campo truncado: não conseguimos avaliar; tratado como casamento desconhecido.
        return (flags & FLAG_UNSUPPORTED_MATCH != 0, false);
    }
    let r = match regra.kind() {
        M_SRC_ZT => Address::from_bytes(f).is_some_and(|a| a == q.zt_source),
        M_DEST_ZT => Address::from_bytes(f).is_some_and(|a| a == q.zt_dest),
        M_VLAN_ID => u16::from_be_bytes([f[0], f[1]]) == 0,
        M_VLAN_PCP | M_VLAN_DEI => f[0] == 0,
        M_MAC_SRC => f[..6] == q.mac_source,
        M_MAC_DEST => f[..6] == q.mac_dest,
        M_IPV4_SRC | M_IPV4_DEST => {
            if ipv4 {
                let em = if regra.kind() == M_IPV4_SRC { 12 } else { 16 };
                contem_v4(&f[..4], f[4], &d[em..em + 4])
            } else {
                hard_no
            }
        }
        M_IPV6_SRC | M_IPV6_DEST => {
            if ipv6 && d.len() >= 40 {
                let em = if regra.kind() == M_IPV6_SRC { 8 } else { 24 };
                contem_v6(&f[..16], f[16], &d[em..em + 16])
            } else {
                hard_no
            }
        }
        M_IP_TOS => {
            let tos = if ipv4 {
                Some(d[1])
            } else if ipv6 && d.len() >= 40 {
                Some(((d[0] << 4) & 0xf0) | ((d[1] >> 4) & 0x0f))
            } else {
                None
            };
            match tos {
                Some(t) => {
                    let t = t & f[0];
                    t >= f[1] && t <= f[2]
                }
                None => hard_no,
            }
        }
        M_IP_PROTOCOL => {
            if ipv4 {
                f[0] == d[9]
            } else if ipv6 {
                match ipv6_payload(d) {
                    Some((_, proto)) => f[0] == proto,
                    None => hard_no,
                }
            } else {
                hard_no
            }
        }
        M_ETHERTYPE => u16::from_be_bytes([f[0], f[1]]) == q.ethertype,
        M_ICMP => casar_icmp(f, q, ipv4, ipv6, hard_yes, hard_no),
        M_SRC_PORT | M_DEST_PORT => {
            let dest = regra.kind() == M_DEST_PORT;
            let de = u16::from_be_bytes([f[0], f[1]]);
            let ate = u16::from_be_bytes([f[2], f[3]]);
            if ipv4 {
                let cabecalho = 4 * usize::from(d[0] & 0x0f);
                porta(d, cabecalho, d[9], dest).is_some_and(|p| p >= de && p <= ate)
            } else if ipv6 {
                match ipv6_payload(d) {
                    Some((pos, proto)) => {
                        porta(d, pos, proto, dest).is_some_and(|p| p > 0 && p >= de && p <= ate)
                    }
                    None => hard_no,
                }
            } else {
                hard_no
            }
        }
        M_CHARACTERISTICS => {
            let mut cf = if q.inbound { CHAR_INBOUND } else { 0 };
            if q.mac_dest[0] & 1 != 0 {
                cf |= CHAR_MULTICAST;
            }
            if q.mac_dest == [0xff; 6] {
                cf |= CHAR_BROADCAST;
            }
            if q.mac_authenticated {
                cf |= CHAR_SENDER_MAC_AUTHENTICATED;
            }
            if ipv4 && d[9] == 0x06 {
                let h = 4 * usize::from(d[0] & 0x0f);
                if let (Some(a), Some(b)) = (d.get(h + 13), d.get(h + 12)) {
                    cf |= u64::from(*a) | (u64::from(*b & 0x0f) << 8);
                }
            } else if ipv6
                && let Some((pos, 0x06)) = ipv6_payload(d)
                && let (Some(a), Some(b)) = (d.get(pos + 13), d.get(pos + 12))
            {
                cf |= u64::from(*a) | (u64::from(*b & 0x0f) << 8);
            }
            let quer = u64::from_be_bytes(f[..8].try_into().expect("8 bytes"));
            cf & quer != 0
        }
        M_FRAME_SIZE => {
            let tam = d.len() + 14;
            let de = usize::from(u16::from_be_bytes([f[0], f[1]]));
            let ate = usize::from(u16::from_be_bytes([f[2], f[3]]));
            tam >= de && tam <= ate
        }
        M_RANDOM => sorteio() <= u32::from_be_bytes(f[..4].try_into().expect("4 bytes")),
        M_TAGS_DIFFERENCE | M_TAGS_AND | M_TAGS_OR | M_TAGS_XOR | M_TAGS_EQUAL => {
            // Não temos tags: a rede que as usa não casa com a nossa ponta.
            hard_no
        }
        M_TAG_SENDER | M_TAG_RECEIVER => {
            if super_accept {
                return (hard_yes, true);
            }
            let do_outro = (regra.kind() == M_TAG_SENDER && q.inbound)
                || (regra.kind() == M_TAG_RECEIVER && !q.inbound);
            if do_outro && regra.kind() == M_TAG_RECEIVER {
                // Saída: ainda não sabemos a tag do destinatário.
                return (hard_yes, true);
            }
            hard_no
        }
        M_INTEGER_RANGE => casar_inteiro(f, d),
        _ => flags & FLAG_UNSUPPORTED_MATCH != 0,
    };
    (r, false)
}

/// O menor tamanho de campo que o casamento precisa.
fn min_field(tipo: u8) -> usize {
    match tipo {
        M_SRC_ZT | M_DEST_ZT => 5,
        M_VLAN_ID | M_ETHERTYPE => 2,
        M_VLAN_PCP | M_VLAN_DEI | M_IP_PROTOCOL => 1,
        M_MAC_SRC | M_MAC_DEST => 6,
        M_IPV4_SRC | M_IPV4_DEST => 5,
        M_IPV6_SRC | M_IPV6_DEST => 17,
        M_IP_TOS | M_ICMP => 3,
        M_SRC_PORT | M_DEST_PORT | M_FRAME_SIZE | M_RANDOM => 4,
        M_CHARACTERISTICS => 8,
        M_TAGS_DIFFERENCE | M_TAGS_AND | M_TAGS_OR | M_TAGS_XOR | M_TAGS_EQUAL | M_TAG_SENDER
        | M_TAG_RECEIVER => 8,
        M_INTEGER_RANGE => 19,
        _ => 0,
    }
}

fn contem_v4(rede: &[u8], mascara: u8, ip: &[u8]) -> bool {
    contem(rede, mascara, ip)
}

fn contem_v6(rede: &[u8], mascara: u8, ip: &[u8]) -> bool {
    contem(rede, mascara, ip)
}

/// O `ip` está na `rede/mascara`? (Prefixo em bits; mais que o tamanho conta como o tamanho.)
fn contem(rede: &[u8], mascara: u8, ip: &[u8]) -> bool {
    let bits = usize::from(mascara).min(rede.len() * 8);
    let inteiros = bits / 8;
    if rede[..inteiros] != ip[..inteiros] {
        return false;
    }
    let resto = bits % 8;
    if resto == 0 {
        return true;
    }
    let m = 0xffu8 << (8 - resto);
    (rede[inteiros] & m) == (ip[inteiros] & m)
}

fn casar_icmp(
    f: &[u8],
    q: &FrameView<'_>,
    ipv4: bool,
    ipv6: bool,
    hard_yes: bool,
    hard_no: bool,
) -> bool {
    let d = q.data;
    let (pos, ok) = if ipv4 {
        let ihl = 4 * usize::from(d[0] & 0x0f);
        (ihl, d[9] == 0x01)
    } else if ipv6 {
        match ipv6_payload(d) {
            Some((pos, proto)) => (pos, proto == 0x3a),
            None => return hard_no,
        }
    } else {
        return hard_no;
    };
    if !ok || d.len() < pos + 2 {
        return hard_no;
    }
    if f[0] != d[pos] {
        return hard_no;
    }
    if f[2] & 0x01 != 0 {
        d[pos + 1] == f[1]
    } else {
        hard_yes
    }
}

/// A porta de origem ou de destino, se o protocolo tem (TCP, UDP, SCTP, UDPLite).
fn porta(d: &[u8], cabecalho: usize, proto: u8, destino: bool) -> Option<u16> {
    if !matches!(proto, 0x06 | 0x11 | 0x84 | 0x88) || d.len() <= cabecalho + 4 {
        return None;
    }
    let pos = cabecalho + if destino { 2 } else { 0 };
    Some(u16::from_be_bytes([d[pos], d[pos + 1]]))
}

/// Pula os cabeçalhos de extensão do IPv6: onde começa a carga e qual o protocolo.
fn ipv6_payload(d: &[u8]) -> Option<(usize, u8)> {
    if d.len() < 40 {
        return None;
    }
    let mut pos = 40usize;
    let mut proto = d[6];
    while pos <= d.len() {
        match proto {
            0 | 43 | 60 | 135 => {
                if pos + 8 > d.len() {
                    return None;
                }
                proto = d[pos];
                pos += usize::from(d[pos + 1]) * 8 + 8;
            }
            _ => return Some((pos, proto)),
        }
    }
    None
}

fn casar_inteiro(f: &[u8], d: &[u8]) -> bool {
    let inicio = u64::from_be_bytes(f[0..8].try_into().expect("8 bytes"));
    let fim = u64::from_be_bytes(f[8..16].try_into().expect("8 bytes"));
    let idx = usize::from(u16::from_be_bytes([f[16], f[17]]));
    let formato = f[18];
    let bits = u32::from(formato & 63) + 1;
    let bytes = bits.div_ceil(8) as usize;
    let mut inteiro: u64 = 0;
    if formato & 0x80 == 0 {
        let mut i = idx + (8 - bytes);
        let eof = i + bytes;
        if eof <= d.len() {
            while i < eof {
                inteiro = (inteiro << 8) | u64::from(d[i]);
                i += 1;
            }
        }
        inteiro &= u64::MAX >> (64 - bits);
    } else {
        let mut i = idx;
        let eof = i + bytes;
        if eof <= d.len() {
            while i < eof {
                inteiro = (inteiro >> 8) | (u64::from(d[i]) << 56);
                i += 1;
            }
        }
        inteiro >>= 64 - bits;
    }
    inteiro >= inicio && inteiro <= fim
}

/// O ARP e o IPv4 são os únicos tipos de quadro que o app manda (o adaptador só fala esses).
pub fn is_supported_ethertype(tipo: u16) -> bool {
    tipo == ETHERTYPE_IPV4 || tipo == ETHERTYPE_ARP
}
