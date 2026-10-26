//! O avaliador das regras da rede (M61, fatia 4): a mesma lógica do `_doZtFilter` do ZeroTier.

use tomni_ui_zerotier::Address;
use tomni_ui_zerotier::netconf::rules::{
    FLAG_UNSUPPORTED_MATCH, FrameView, Outcome, Rule, Verdict, evaluate, parse, serialize,
};

const NOSSO: u64 = 0x0a0b0c0d0e;
const OUTRO: u64 = 0x1112131415;

fn ipv4(proto: u8, origem: [u8; 4], destino: [u8; 4], porta_dest: u16) -> Vec<u8> {
    let mut p = vec![0u8; 28];
    p[0] = 0x45;
    p[9] = proto;
    p[12..16].copy_from_slice(&origem);
    p[16..20].copy_from_slice(&destino);
    p[20..22].copy_from_slice(&40000u16.to_be_bytes());
    p[22..24].copy_from_slice(&porta_dest.to_be_bytes());
    p
}

fn quadro(dados: &[u8], entrando: bool) -> FrameView<'_> {
    FrameView {
        zt_source: Address::new(OUTRO),
        zt_dest: Address::new(NOSSO),
        mac_source: [2, 0, 0, 0, 0, 1],
        mac_dest: [2, 0, 0, 0, 0, 2],
        ethertype: 0x0800,
        data: dados,
        inbound: entrando,
        mac_authenticated: true,
    }
}

fn avalia(regras: &[Rule], q: &FrameView<'_>) -> Outcome {
    evaluate(regras, q, Address::new(NOSSO), 0, || 0)
}

#[test]
fn sem_regra_aceitando_o_quadro_cai() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    assert_eq!(avalia(&[], &quadro(&d, true)).verdict, Verdict::Drop);
    assert_eq!(
        avalia(&[Rule::drop()], &quadro(&d, true)).verdict,
        Verdict::Drop
    );
}

#[test]
fn aceitar_tudo_aceita() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    assert_eq!(
        avalia(&[Rule::accept()], &quadro(&d, true)).verdict,
        Verdict::Accept
    );
}

#[test]
fn o_tipo_do_quadro_e_a_porta_filtram() {
    // Aceita só TCP na porta 22.
    let regras = [
        Rule::ethertype(0x0800, true),
        Rule::drop(),
        Rule::ip_protocol(6, true),
        Rule::drop(),
        Rule::dest_port(22, 22, true),
        Rule::drop(),
        Rule::accept(),
    ];
    let ssh = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let web = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 80);
    let udp = ipv4(17, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    assert_eq!(
        avalia(&regras, &quadro(&ssh, true)).verdict,
        Verdict::Accept
    );
    assert_eq!(avalia(&regras, &quadro(&web, true)).verdict, Verdict::Drop);
    assert_eq!(avalia(&regras, &quadro(&udp, true)).verdict, Verdict::Drop);
    // Um quadro ARP não é IPv4: a primeira regra (negada) o descarta.
    let mut arp = quadro(&ssh, true);
    arp.ethertype = 0x0806;
    assert_eq!(avalia(&regras, &arp).verdict, Verdict::Drop);
}

#[test]
fn a_rede_ipv4_de_origem_casa_pelo_prefixo() {
    let mut campo = vec![10, 61, 0, 0, 16];
    let casa = Rule {
        t: 31,
        field: std::mem::take(&mut campo),
    };
    let regras = [casa, Rule::accept()];
    let dentro = ipv4(1, [10, 61, 9, 9], [10, 61, 0, 1], 0);
    let fora = ipv4(1, [10, 62, 9, 9], [10, 61, 0, 1], 0);
    assert_eq!(
        avalia(&regras, &quadro(&dentro, true)).verdict,
        Verdict::Accept
    );
    assert_eq!(avalia(&regras, &quadro(&fora, true)).verdict, Verdict::Drop);
}

#[test]
fn ou_junta_os_casamentos_e_e_exige_todos() {
    let ssh = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let dns = ipv4(17, [10, 0, 0, 1], [10, 0, 0, 2], 53);
    let web = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 80);
    // 22 OU 53 → aceita.
    let mut ou = Rule::dest_port(53, 53, false);
    ou.t |= 0x40;
    let regras = [Rule::dest_port(22, 22, false), ou, Rule::accept()];
    assert_eq!(
        avalia(&regras, &quadro(&ssh, true)).verdict,
        Verdict::Accept
    );
    assert_eq!(
        avalia(&regras, &quadro(&dns, true)).verdict,
        Verdict::Accept
    );
    assert_eq!(avalia(&regras, &quadro(&web, true)).verdict, Verdict::Drop);
}

#[test]
fn a_negacao_inverte_o_casamento() {
    // "Não é a porta 22 → descarta": a 22 passa, o resto cai.
    let ssh = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let web = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 80);
    let regras = [Rule::dest_port(22, 22, true), Rule::drop(), Rule::accept()];
    assert_eq!(
        avalia(&regras, &quadro(&ssh, true)).verdict,
        Verdict::Accept
    );
    assert_eq!(avalia(&regras, &quadro(&web, true)).verdict, Verdict::Drop);
}

#[test]
fn break_para_a_avaliacao() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let regras = [Rule::action(5), Rule::accept()];
    assert_eq!(avalia(&regras, &quadro(&d, true)).verdict, Verdict::Drop);
}

#[test]
fn caracteristicas_dizem_entrada_broadcast_e_multicast() {
    let d = ipv4(1, [10, 0, 0, 1], [10, 0, 0, 255], 0);
    let so_entrada = Rule {
        t: 41,
        field: 0x8000_0000_0000_0000u64.to_be_bytes().to_vec(),
    };
    let regras = [so_entrada, Rule::accept()];
    assert_eq!(avalia(&regras, &quadro(&d, true)).verdict, Verdict::Accept);
    assert_eq!(avalia(&regras, &quadro(&d, false)).verdict, Verdict::Drop);

    let broadcast = Rule {
        t: 41,
        field: 0x2000_0000_0000_0000u64.to_be_bytes().to_vec(),
    };
    let regras = [broadcast, Rule::drop(), Rule::accept()];
    let mut q = quadro(&d, true);
    assert_eq!(avalia(&regras, &q).verdict, Verdict::Accept);
    q.mac_dest = [0xff; 6];
    assert_eq!(avalia(&regras, &q).verdict, Verdict::Drop);
}

#[test]
fn tamanho_do_quadro() {
    let d = ipv4(1, [10, 0, 0, 1], [10, 0, 0, 2], 0);
    let mut campo = 0u16.to_be_bytes().to_vec();
    campo.extend_from_slice(&40u16.to_be_bytes());
    let pequeno = Rule {
        t: 42,
        field: campo,
    };
    let regras = [pequeno, Rule::accept()];
    // 28 bytes de IP + 14 do Ethernet = 42, acima de 40.
    assert_eq!(avalia(&regras, &quadro(&d, true)).verdict, Verdict::Drop);
}

#[test]
fn redirecionar_e_copiar() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let alvo = Address::new(0x2122232425);
    let r = avalia(&[Rule::redirect(alvo)], &quadro(&d, false));
    assert_eq!(r.verdict, Verdict::Redirect(alvo));

    let r = avalia(&[Rule::tee(alvo, 0), Rule::accept()], &quadro(&d, false));
    assert_eq!(r.verdict, Verdict::Accept);
    assert_eq!(r.copy, Some((alvo, d.len() + 14, false)));

    // Copiar para quem já é a origem não faz nada.
    let r = avalia(
        &[Rule::tee(Address::new(OUTRO), 0), Rule::accept()],
        &quadro(&d, false),
    );
    assert_eq!(r.copy, None);
}

#[test]
fn casamento_desconhecido_segue_a_flag_da_config() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let desconhecido = Rule {
        t: 62,
        field: vec![1, 2, 3],
    };
    let regras = [desconhecido, Rule::accept()];
    let q = quadro(&d, true);
    assert_eq!(
        evaluate(&regras, &q, Address::new(NOSSO), 0, || 0).verdict,
        Verdict::Drop
    );
    assert_eq!(
        evaluate(
            &regras,
            &q,
            Address::new(NOSSO),
            FLAG_UNSUPPORTED_MATCH,
            || 0
        )
        .verdict,
        Verdict::Accept
    );
}

#[test]
fn sorteio_aceita_so_uma_fracao() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let metade = Rule {
        t: 43,
        field: (u32::MAX / 2).to_be_bytes().to_vec(),
    };
    let regras = [metade, Rule::accept()];
    let q = quadro(&d, true);
    assert_eq!(
        evaluate(&regras, &q, Address::new(NOSSO), 0, || 10).verdict,
        Verdict::Accept
    );
    assert_eq!(
        evaluate(&regras, &q, Address::new(NOSSO), 0, || u32::MAX).verdict,
        Verdict::Drop
    );
}

#[test]
fn tags_que_nao_temos_nao_casam() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let igual = Rule {
        t: 48,
        field: vec![0, 0, 0, 1, 0, 0, 0, 2],
    };
    assert_eq!(
        avalia(&[igual, Rule::accept()], &quadro(&d, true)).verdict,
        Verdict::Drop
    );
}

#[test]
fn campo_truncado_nao_derruba() {
    let d = ipv4(6, [10, 0, 0, 1], [10, 0, 0, 2], 22);
    let curto = Rule {
        t: 31,
        field: vec![10],
    };
    assert_eq!(
        avalia(&[curto, Rule::accept()], &quadro(&d, true)).verdict,
        Verdict::Drop
    );
}

#[test]
fn as_regras_vao_e_voltam_pelo_fio() {
    let regras = vec![
        Rule::ethertype(0x0806, false),
        Rule::accept(),
        Rule::dest_port(1, 1024, true),
        Rule::drop(),
    ];
    assert_eq!(parse(&serialize(&regras)).unwrap(), regras);
    // O fim no meio de uma regra.
    let mut bytes = serialize(&regras);
    bytes.pop();
    assert!(parse(&bytes).is_none());
}

#[test]
fn quadro_vazio_nao_derruba_o_avaliador() {
    let q = quadro(&[], true);
    let regras = [
        Rule::ip_protocol(6, false),
        Rule::dest_port(1, 2, false),
        Rule::accept(),
    ];
    assert_eq!(avalia(&regras, &q).verdict, Verdict::Drop);
}
