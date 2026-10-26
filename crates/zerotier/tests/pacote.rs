//! Os pacotes do VL1 contra nós oficiais de verdade: `tests/fixtures/captura/captura.pcap` saiu de
//! `testes/zerotier/capturar.sh` (nós 1.14.2 e 1.4.6 em contêineres, com as identidades das
//! fixtures). Os testes decifram o que foi capturado, conferem os campos e armam de novo, byte
//! a byte igual.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use tomni_ui_zerotier::crypto::PeerKey;
use tomni_ui_zerotier::crypto::armor::extended_dearmor;
use tomni_ui_zerotier::crypto::gmac_siv;
use tomni_ui_zerotier::crypto::salsa;
use tomni_ui_zerotier::packet::{Cipher, Packet, Verb};
use tomni_ui_zerotier::{Address, Identity};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// As três identidades do `gerar-fixtures.sh`, pelo endereço.
fn identidades() -> BTreeMap<u64, Identity> {
    (1..=3)
        .map(|n| {
            let texto =
                fs::read_to_string(fixtures().join(format!("identidades/id{n}.secret"))).unwrap();
            let id = Identity::from_secret_str(texto.trim()).unwrap();
            (id.address().as_u64(), id)
        })
        .collect()
}

/// Um datagrama UDP da captura: portas e a carga.
struct Datagrama {
    de_porta: u16,
    para_porta: u16,
    carga: Vec<u8>,
}

/// Lê o pcap (formato clássico, little-endian, Ethernet) e devolve os datagramas UDP/IPv4.
fn ler_pcap() -> Vec<Datagrama> {
    let dados = fs::read(fixtures().join("captura/captura.pcap")).unwrap();
    assert_eq!(&dados[..4], &[0xd4, 0xc3, 0xb2, 0xa1], "pcap little-endian");
    let tipo_de_enlace = u32::from_le_bytes(dados[20..24].try_into().unwrap());
    assert_eq!(tipo_de_enlace, 1, "Ethernet");
    let mut saida = Vec::new();
    let mut pos = 24;
    while pos + 16 <= dados.len() {
        let incluido = u32::from_le_bytes(dados[pos + 8..pos + 12].try_into().unwrap()) as usize;
        let quadro = &dados[pos + 16..pos + 16 + incluido];
        pos += 16 + incluido;
        // Ethernet (14) + IPv4 + UDP (8); o resto (ARP, IPv6) não interessa.
        if quadro.len() < 14 + 20 + 8 || quadro[12..14] != [0x08, 0x00] {
            continue;
        }
        let ip = &quadro[14..];
        let ihl = usize::from(ip[0] & 0x0f) * 4;
        if ip[9] != 17 {
            continue;
        }
        let udp = &ip[ihl..];
        let tamanho = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
        saida.push(Datagrama {
            de_porta: u16::from_be_bytes([udp[0], udp[1]]),
            para_porta: u16::from_be_bytes([udp[2], udp[3]]),
            carga: udp[8..tamanho].to_vec(),
        });
    }
    saida
}

/// Um pacote da captura já identificado: quem mandou, para quem e os bytes como vieram.
struct Capturado {
    origem: u64,
    destino: u64,
    bytes: Vec<u8>,
}

fn pacotes_inteiros() -> Vec<Capturado> {
    let ids = identidades();
    ler_pcap()
        .into_iter()
        .filter(|d| d.carga.len() >= 28 && d.carga[13] != 0xff)
        .filter_map(|d| {
            let destino = Address::from_bytes(&d.carga[8..13])?.as_u64();
            let origem = Address::from_bytes(&d.carga[13..18])?.as_u64();
            (ids.contains_key(&destino) && ids.contains_key(&origem)).then_some(Capturado {
                origem,
                destino,
                bytes: d.carga,
            })
        })
        .collect()
}

/// A chave do par vista pelo lado de `destino`.
fn chave(ids: &BTreeMap<u64, Identity>, c: &Capturado) -> PeerKey {
    let acordo = ids[&c.destino].agree(&ids[&c.origem]).unwrap();
    PeerKey::from_agreement(&acordo)
}

/// Tira as duas camadas, se houver, e devolve o pacote em claro.
fn abrir(ids: &BTreeMap<u64, Identity>, c: &Capturado) -> Option<Packet> {
    let mut bytes = c.bytes.clone();
    let p = Packet::from_bytes(bytes.clone()).unwrap();
    if p.cipher() == Cipher::None && p.extended_armor() {
        extended_dearmor(&mut bytes, &ids[&c.destino]).ok()?;
    }
    salsa::dearmor(&chave(ids, c).salsa, &mut bytes).ok()?;
    Some(Packet::from_bytes(bytes).unwrap())
}

#[test]
fn a_captura_tem_hello_nos_dois_sentidos_e_a_resposta() {
    let pacotes = pacotes_inteiros();
    assert!(pacotes.len() >= 6, "só {} pacotes da rede", pacotes.len());
    let ids = identidades();
    let mut hellos = 0;
    let mut oks = 0;
    for c in &pacotes {
        let Some(p) = abrir(&ids, c) else { continue };
        match p.verb() {
            Verb::Hello => hellos += 1,
            Verb::Ok => oks += 1,
            _ => {}
        }
    }
    assert!(hellos >= 2, "HELLO decifrados: {hellos}");
    assert!(oks >= 1, "OK decifrados: {oks}");
}

#[test]
fn o_hello_traz_o_protocolo_a_versao_e_a_identidade_de_quem_manda() {
    let ids = identidades();
    let mut visto = false;
    for c in pacotes_inteiros() {
        let Some(p) = abrir(&ids, &c) else { continue };
        if p.verb() != Verb::Hello {
            continue;
        }
        let carga = p.payload();
        // protocolo, versão maior, menor, revisão (2 bytes), horário (8 bytes) e a identidade.
        assert!((4..=20).contains(&carga[0]), "protocolo {}", carga[0]);
        let (id, _) = Identity::from_wire(&carga[13..]).expect("identidade no HELLO");
        assert_eq!(id.to_wire(), carga[13..13 + id.to_wire().len()]);
        assert_eq!(id.address().as_u64(), c.origem);
        assert!(id.validate());
        visto = true;
    }
    assert!(visto, "nenhum HELLO decifrado");
}

#[test]
fn armar_de_novo_o_pacote_decifrado_da_os_mesmos_bytes() {
    let ids = identidades();
    let mut conferidos = 0;
    for c in pacotes_inteiros() {
        let p = Packet::from_bytes(c.bytes.clone()).unwrap();
        // A armadura estendida usa uma chave efêmera nova a cada vez: só a Salsa é repetível.
        if p.cipher() == Cipher::None && p.extended_armor() {
            continue;
        }
        let mut bytes = c.bytes.clone();
        if salsa::dearmor(&chave(&ids, &c).salsa, &mut bytes).is_err() {
            continue;
        }
        let cifrado = p.cipher() == Cipher::Salsa2012;
        salsa::armor(&chave(&ids, &c).salsa, &mut bytes, cifrado);
        assert_eq!(bytes, c.bytes, "pacote de {:010x}", c.origem);
        conferidos += 1;
    }
    assert!(conferidos >= 1, "nenhum pacote Salsa para conferir");
}

#[test]
fn um_byte_trocado_na_captura_e_recusado() {
    let ids = identidades();
    for c in pacotes_inteiros() {
        let p = Packet::from_bytes(c.bytes.clone()).unwrap();
        if p.cipher() != Cipher::Salsa2012 {
            continue;
        }
        let mut bytes = c.bytes.clone();
        let ultimo = bytes.len() - 1;
        bytes[ultimo] ^= 1;
        assert!(salsa::dearmor(&chave(&ids, &c).salsa, &mut bytes).is_err());
        return;
    }
    panic!("nenhum pacote Salsa na captura");
}

#[test]
fn a_flag_0x80_com_a_suite_1_nao_e_armadura_estendida() {
    // O 1.14.2 deixa o bit ligado no OK(HELLO) em Salsa (suíte 1); só a suíte 0 tem a camada.
    let ids = identidades();
    for c in pacotes_inteiros() {
        let p = Packet::from_bytes(c.bytes.clone()).unwrap();
        if p.cipher() == Cipher::Salsa2012 && p.extended_armor() {
            let mut bytes = c.bytes.clone();
            salsa::dearmor(&chave(&ids, &c).salsa, &mut bytes)
                .expect("a suíte 1 não tem armadura estendida por cima");
            return;
        }
    }
    eprintln!("a captura não tem pacote Salsa com o bit 0x80");
}

#[test]
fn as_portas_da_captura_incluem_a_9993() {
    let datagramas = ler_pcap();
    assert!(
        datagramas
            .iter()
            .any(|d| d.de_porta == 9993 || d.para_porta == 9993)
    );
}

#[test]
fn os_pacotes_aes_gmac_siv_da_captura_decifram_e_conferem() {
    let ids = identidades();
    let mut abertos = 0;
    for c in pacotes_inteiros() {
        let p = Packet::from_bytes(c.bytes.clone()).unwrap();
        if p.cipher() != Cipher::AesGmacSiv {
            continue;
        }
        let mut bytes = c.bytes.clone();
        gmac_siv::dearmor(&chave(&ids, &c).aes, &mut bytes)
            .unwrap_or_else(|e| panic!("AES de {:010x} para {:010x}: {e}", c.origem, c.destino));
        // O ID que viaja é a metade da etiqueta e continua o mesmo depois de abrir.
        assert_eq!(bytes[..8], c.bytes[..8]);
        let aberto = Packet::from_bytes(bytes).unwrap();
        assert!(
            matches!(
                aberto.verb(),
                Verb::Ok
                    | Verb::Echo
                    | Verb::Frame
                    | Verb::ExtFrame
                    | Verb::MulticastGather
                    | Verb::MulticastLike
                    | Verb::NetworkConfigRequest
                    | Verb::NetworkCredentials
                    | Verb::Whois
                    | Verb::Rendezvous
                    | Verb::PushDirectPaths
                    | Verb::Error
                    | Verb::Nop
            ),
            "verbo inesperado {:?}",
            aberto.verb()
        );
        abertos += 1;
    }
    assert!(abertos >= 4, "só {abertos} pacotes AES na captura");
}

#[test]
fn um_byte_trocado_num_pacote_aes_e_recusado() {
    let ids = identidades();
    for c in pacotes_inteiros() {
        let p = Packet::from_bytes(c.bytes.clone()).unwrap();
        if p.cipher() != Cipher::AesGmacSiv {
            continue;
        }
        let chaves = chave(&ids, &c).aes;
        for posicao in [0usize, 10, 19, c.bytes.len() - 1] {
            let mut bytes = c.bytes.clone();
            bytes[posicao] ^= 1;
            assert!(
                gmac_siv::dearmor(&chaves, &mut bytes).is_err(),
                "byte {posicao} trocado passou"
            );
        }
        // Os saltos não entram na autenticação.
        let mut bytes = c.bytes.clone();
        bytes[18] = (bytes[18] & !7) | 3;
        gmac_siv::dearmor(&chaves, &mut bytes).expect("os saltos mudam no caminho");
        return;
    }
    panic!("nenhum pacote AES na captura");
}
