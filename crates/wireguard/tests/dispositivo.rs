//! O dispositivo WireGuard de ponta a ponta, sem rede: dois `Device` trocam datagramas em
//! memória e o relógio é um `Instant` que o teste avança.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use tomni_ui_wireguard::{
    Cidr, Device, Event, KEEPALIVE_TIMEOUT, PeerConfig, PeerError, PresharedKey, PrivateKey,
    REJECT_AFTER_TIME, REKEY_AFTER_TIME, REKEY_ATTEMPT_TIME, REKEY_TIMEOUT, SendError,
    TAMANHO_COOKIE, TAMANHO_INICIACAO, TAMANHO_KEEPALIVE, TAMANHO_RESPOSTA,
};

const IP_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
const IP_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

/// Um lado da conversa: o dispositivo e o que ele produziu.
struct Lado {
    dev: Device,
    addr: SocketAddr,
    /// Os datagramas a mandar, na ordem.
    saida: Vec<(SocketAddr, Vec<u8>)>,
    /// Os pacotes IP entregues à pilha.
    recebidos: Vec<Vec<u8>>,
    erros: Vec<PeerError>,
}

impl Lado {
    fn drenar(&mut self, now: Instant) {
        loop {
            let ev = self.dev.poll(now);
            let (saida, recebidos, erros) = (&mut self.saida, &mut self.recebidos, &mut self.erros);
            let mais = match ev {
                Event::Nothing => false,
                Event::Transmit { to, data } => {
                    saida.push((to, data.to_vec()));
                    true
                }
                Event::Ip { data, .. } => {
                    recebidos.push(data.to_vec());
                    true
                }
                Event::PeerError { error, .. } => {
                    erros.push(error);
                    true
                }
            };
            if !mais {
                break;
            }
        }
    }

    fn receber(&mut self, datagrama: &[u8], de: SocketAddr, now: Instant) {
        let ev = self.dev.receive(datagrama, de, now);
        let (saida, recebidos, erros) = (&mut self.saida, &mut self.recebidos, &mut self.erros);
        match ev {
            Event::Nothing => {}
            Event::Transmit { to, data } => saida.push((to, data.to_vec())),
            Event::Ip { data, .. } => recebidos.push(data.to_vec()),
            Event::PeerError { error, .. } => erros.push(error),
        }
        self.drenar(now);
    }
}

/// Os dois lados, ligados em memória.
struct Rede {
    a: Lado,
    b: Lado,
    t0: Instant,
}

/// O que fazer com cada datagrama que passa de um lado a outro.
type Filtro<'a> = dyn FnMut(bool, &[u8]) -> bool + 'a;

impl Rede {
    fn nova(psk: Option<PresharedKey>) -> Rede {
        Self::com(psk, None)
    }

    fn com(psk: Option<PresharedKey>, keepalive_de_a: Option<u16>) -> Rede {
        let t0 = Instant::now();
        let ka = PrivateKey::generate().unwrap();
        let kb = PrivateKey::generate().unwrap();
        let (pub_a, pub_b) = (ka.public_key(), kb.public_key());
        let addr_a: SocketAddr = "192.0.2.1:51820".parse().unwrap();
        let addr_b: SocketAddr = "192.0.2.2:51820".parse().unwrap();
        let mut a = Device::new(ka, t0).unwrap();
        let mut b = Device::new(kb, t0).unwrap();
        let mut config_b = PeerConfig::new(pub_b);
        let mut config_a = PeerConfig::new(pub_a);
        if let Some(psk) = psk {
            config_b = config_b.with_preshared_key(psk.clone());
            config_a = config_a.with_preshared_key(psk);
        }
        if let Some(s) = keepalive_de_a {
            config_b = config_b.with_persistent_keepalive_secs(s);
        }
        let faixa = |ip| vec![Cidr::new(IpAddr::V4(ip), 32).unwrap()];
        a.add_peer(config_b, Some(addr_b), &faixa(IP_B), t0)
            .unwrap();
        // O B não conhece o endereço do A: aprende quando o A iniciar.
        b.add_peer(config_a, None, &faixa(IP_A), t0).unwrap();
        let lado = |dev, addr| Lado {
            dev,
            addr,
            saida: Vec::new(),
            recebidos: Vec::new(),
            erros: Vec::new(),
        };
        Rede {
            a: lado(a, addr_a),
            b: lado(b, addr_b),
            t0,
        }
    }

    fn em(&self, segundos: f64) -> Instant {
        self.t0 + Duration::from_secs_f64(segundos)
    }

    /// Roda os dois lados e leva os datagramas de um a outro até acalmar. O filtro recebe se o
    /// datagrama vai do A ao B e os bytes; devolver `false` perde o datagrama.
    fn trocar(&mut self, now: Instant, filtro: &mut Filtro<'_>) {
        for _ in 0..40 {
            self.a.drenar(now);
            self.b.drenar(now);
            if self.a.saida.is_empty() && self.b.saida.is_empty() {
                return;
            }
            let de_a: Vec<_> = self.a.saida.drain(..).collect();
            for (para, dados) in de_a {
                if para == self.b.addr && filtro(true, &dados) {
                    let origem = self.a.addr;
                    self.b.receber(&dados, origem, now);
                }
            }
            let de_b: Vec<_> = self.b.saida.drain(..).collect();
            for (para, dados) in de_b {
                if para == self.a.addr && filtro(false, &dados) {
                    let origem = self.b.addr;
                    self.a.receber(&dados, origem, now);
                }
            }
        }
        panic!("a conversa não acalmou em 40 rodadas");
    }

    /// Anda o relógio de `de` a `ate` segundos, de um em um, trocando datagramas a cada passo.
    fn andar(&mut self, de: u32, ate: u32, filtro: &mut Filtro<'_>) {
        for s in de..=ate {
            self.trocar(self.em(f64::from(s)), filtro);
        }
    }

    fn trocar_tudo(&mut self, now: Instant) {
        self.trocar(now, &mut |_, _| true);
    }
}

fn pacote(origem: Ipv4Addr, destino: Ipv4Addr, carga: &[u8]) -> Vec<u8> {
    let total = 20 + carga.len();
    let mut p = vec![0u8; 20];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&origem.octets());
    p[16..20].copy_from_slice(&destino.octets());
    p.extend_from_slice(carga);
    p
}

fn tipo(dados: &[u8]) -> u32 {
    u32::from_le_bytes(dados[0..4].try_into().unwrap())
}

#[test]
fn o_primeiro_pacote_dispara_o_handshake_e_chega_inteiro() {
    for psk in [None, Some(PresharedKey::generate().unwrap())] {
        let mut r = Rede::nova(psk);
        let p = pacote(IP_A, IP_B, b"ola, B");
        r.a.dev.send(&p).unwrap();
        r.trocar_tudo(r.em(0.0));
        assert_eq!(r.b.recebidos, vec![p]);
        // Agora o B responde pelo mesmo túnel.
        let volta = pacote(IP_B, IP_A, b"ola, A");
        r.b.dev.send(&volta).unwrap();
        r.trocar_tudo(r.em(1.0));
        assert_eq!(r.a.recebidos, vec![volta]);
    }
}

#[test]
fn o_receptor_aprende_o_endpoint_de_quem_iniciou() {
    let mut r = Rede::nova(None);
    let pub_a = r.a.dev.public_key();
    assert_eq!(r.b.dev.endpoint(&pub_a), None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"x")).unwrap();
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.dev.endpoint(&pub_a), Some(r.a.addr));
}

#[test]
fn os_pacotes_que_esperam_a_sessao_saem_em_ordem() {
    let mut r = Rede::nova(None);
    let ps: Vec<_> = (0..5u8).map(|i| pacote(IP_A, IP_B, &[i; 30])).collect();
    for p in &ps {
        r.a.dev.send(p).unwrap();
    }
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.recebidos, ps);
}

#[test]
fn sem_rota_o_pacote_e_recusado() {
    let mut r = Rede::nova(None);
    let fora = Ipv4Addr::new(8, 8, 8, 8);
    assert_eq!(
        r.a.dev.send(&pacote(IP_A, fora, b"x")),
        Err(SendError::NoRoute(IpAddr::V4(fora)))
    );
    assert_eq!(r.a.dev.send(&[1, 2, 3]), Err(SendError::NotIp));
}

#[test]
fn mac1_errado_e_descartado_em_silencio() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"x")).unwrap();
    r.a.drenar(r.em(0.0));
    let (_, mut ini) = r.a.saida.remove(0);
    assert_eq!(ini.len(), TAMANHO_INICIACAO);
    let n = ini.len();
    ini[n - 20] ^= 1; // dentro do mac1
    let origem = r.a.addr;
    r.b.receber(&ini, origem, r.em(0.0));
    assert!(r.b.saida.is_empty());
    assert!(r.b.recebidos.is_empty());
}

#[test]
fn iniciacao_de_desconhecido_e_descartada() {
    let mut r = Rede::nova(None);
    // Um terceiro, que o B não conhece, tenta falar com o B.
    let t0 = r.t0;
    let intruso_chave = PrivateKey::generate().unwrap();
    let mut intruso = Device::new(intruso_chave, t0).unwrap();
    let pub_b = r.b.dev.public_key();
    intruso
        .add_peer(
            PeerConfig::new(pub_b),
            Some(r.b.addr),
            &[Cidr::new(IpAddr::V4(IP_B), 32).unwrap()],
            t0,
        )
        .unwrap();
    intruso.send(&pacote(IP_A, IP_B, b"x")).unwrap();
    let mut ini = None;
    if let Event::Transmit { data, .. } = intruso.poll(t0) {
        ini = Some(data.to_vec());
    }
    let ini = ini.expect("o intruso devia iniciar");
    let origem = "198.51.100.9:4000".parse().unwrap();
    r.b.receber(&ini, origem, t0);
    assert!(r.b.saida.is_empty());
}

#[test]
fn iniciacao_repetida_e_recusada() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"x")).unwrap();
    r.a.drenar(r.em(0.0));
    let (_, ini) = r.a.saida[0].clone();
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.recebidos.len(), 1);
    // Um atacante repete a mesma iniciação, depois: o carimbo de tempo não é maior.
    let origem = r.a.addr;
    r.b.receber(&ini, origem, r.em(30.0));
    assert!(
        r.b.saida.iter().all(|(_, d)| d.len() != TAMANHO_RESPOSTA),
        "não pode haver resposta a uma repetição"
    );
}

#[test]
fn a_iniciacao_perdida_e_retransmitida_apos_o_rekey_timeout() {
    let mut r = Rede::nova(None);
    let p = pacote(IP_A, IP_B, b"x");
    r.a.dev.send(&p).unwrap();
    // A primeira iniciação se perde.
    let mut primeira = true;
    r.trocar(r.em(0.0), &mut |de_a, d| {
        if de_a && primeira && tipo(d) == 1 {
            primeira = false;
            return false;
        }
        true
    });
    assert!(r.b.recebidos.is_empty());
    // Antes do prazo nada acontece.
    r.trocar_tudo(r.em(3.0));
    assert!(r.b.recebidos.is_empty());
    // Depois do `REKEY_TIMEOUT` mais a variação, a iniciação sai de novo.
    r.trocar_tudo(r.em(REKEY_TIMEOUT.as_secs_f64() + 0.5));
    assert_eq!(r.b.recebidos, vec![p]);
}

#[test]
fn sob_carga_o_cookie_obriga_a_repetir_com_mac2() {
    let mut r = Rede::nova(None);
    r.b.dev.set_under_load(true);
    let p = pacote(IP_A, IP_B, b"x");
    r.a.dev.send(&p).unwrap();
    // O B responde à iniciação com um cookie (64 bytes), não com a resposta (92).
    let mut tipos = Vec::new();
    r.trocar(r.em(0.0), &mut |_, d| {
        tipos.push(d.len());
        true
    });
    assert_eq!(tipos, vec![TAMANHO_INICIACAO, TAMANHO_COOKIE]);
    assert!(r.b.recebidos.is_empty());
    // Na retransmissão o A manda o `mac2` e o B aceita, mesmo sob carga.
    let mut tamanhos = Vec::new();
    r.trocar(r.em(REKEY_TIMEOUT.as_secs_f64() + 0.5), &mut |_, d| {
        tamanhos.push(d.len());
        true
    });
    assert!(tamanhos.contains(&TAMANHO_RESPOSTA), "{tamanhos:?}");
    assert_eq!(r.b.recebidos, vec![p]);
}

#[test]
fn passados_120_s_quem_iniciou_troca_a_chave_ao_enviar() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"1")).unwrap();
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.recebidos.len(), 1);
    // Antes de 120 s só vão dados.
    let mut so_dados = true;
    r.a.dev.send(&pacote(IP_A, IP_B, b"2")).unwrap();
    r.andar(1, 100, &mut |_, d| {
        so_dados &= tipo(d) == 4;
        true
    });
    assert!(so_dados);
    // Depois, o próprio envio dispara um handshake novo.
    let mut viu_iniciacao = false;
    r.a.dev.send(&pacote(IP_A, IP_B, b"3")).unwrap();
    let depois = REKEY_AFTER_TIME.as_secs_f64() + 1.0;
    r.trocar(r.em(depois), &mut |_, d| {
        viu_iniciacao |= tipo(d) == 1;
        true
    });
    assert!(viu_iniciacao);
    assert_eq!(r.b.recebidos.len(), 3);
    // O túnel continua inteiro depois da troca.
    r.b.dev.send(&pacote(IP_B, IP_A, b"volta")).unwrap();
    r.trocar_tudo(r.em(depois + 1.0));
    assert_eq!(r.a.recebidos.len(), 1);
}

#[test]
fn passados_180_s_a_chave_morre_e_o_envio_recomeca_o_handshake() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"1")).unwrap();
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.recebidos.len(), 1);
    // A rede some por mais que `REJECT_AFTER_TIME`.
    let depois = REJECT_AFTER_TIME.as_secs_f64() + 5.0;
    r.a.dev.send(&pacote(IP_A, IP_B, b"2")).unwrap();
    r.a.drenar(r.em(depois));
    let primeiro = r.a.saida.first().expect("devia haver saída").1.clone();
    assert_eq!(
        tipo(&primeiro),
        1,
        "com a chave morta só uma iniciação pode sair"
    );
    r.a.saida.clear();
}

#[test]
fn sem_o_que_responder_o_receptor_manda_keepalive_apos_10_s() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"1")).unwrap();
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.recebidos.len(), 1);
    // O B recebeu dados e não tem o que responder: depois de `KEEPALIVE_TIMEOUT`, keepalive.
    let mut keepalives = 0;
    let antes = KEEPALIVE_TIMEOUT.as_secs_f64() - 1.0;
    r.trocar(r.em(antes), &mut |de_a, d| {
        keepalives += usize::from(!de_a && d.len() == TAMANHO_KEEPALIVE);
        true
    });
    assert_eq!(keepalives, 0);
    r.trocar(
        r.em(KEEPALIVE_TIMEOUT.as_secs_f64() + 0.1),
        &mut |de_a, d| {
            keepalives += usize::from(!de_a && d.len() == TAMANHO_KEEPALIVE);
            true
        },
    );
    assert_eq!(keepalives, 1);
    // O keepalive não vira pacote para a pilha.
    assert!(r.a.recebidos.is_empty());
}

#[test]
fn o_keepalive_persistente_sai_no_intervalo() {
    let mut r = Rede::com(None, Some(25));
    // Com o keepalive persistente a sessão nasce sem precisar de dados.
    r.trocar_tudo(r.em(0.0));
    let mut keepalives = 0;
    for s in [10.0, 24.0] {
        r.trocar(r.em(s), &mut |de_a, d| {
            keepalives += usize::from(de_a && d.len() == TAMANHO_KEEPALIVE);
            true
        });
    }
    assert_eq!(keepalives, 0, "antes dos 25 s não sai nada");
    r.trocar(r.em(25.5), &mut |de_a, d| {
        keepalives += usize::from(de_a && d.len() == TAMANHO_KEEPALIVE);
        true
    });
    assert_eq!(keepalives, 1);
}

#[test]
fn sem_resposta_o_par_e_dado_como_inalcancavel_em_90_s() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"x")).unwrap();
    // Toda a rede está fora do ar.
    let mut segundos = 0.0;
    while segundos <= REKEY_ATTEMPT_TIME.as_secs_f64() + 10.0 {
        r.trocar(r.em(segundos), &mut |_, _| false);
        segundos += 1.0;
        if !r.a.erros.is_empty() {
            break;
        }
    }
    assert_eq!(r.a.erros, vec![PeerError::HandshakeTimeout]);
    assert!(
        segundos >= REKEY_ATTEMPT_TIME.as_secs_f64(),
        "desistiu cedo, em {segundos} s"
    );
}

#[test]
fn o_pacote_com_origem_fora_das_faixas_do_par_e_descartado() {
    let mut r = Rede::nova(None);
    // O A manda um pacote com a origem trocada: o B só aceita 10.0.0.1/32 vindo do A. Para
    // isso o A precisa ter essa origem fora da rota dele, então o roteamento é pelo destino.
    let falso = pacote(Ipv4Addr::new(10, 9, 9, 9), IP_B, b"x");
    r.a.dev.send(&falso).unwrap();
    r.trocar_tudo(r.em(0.0));
    assert!(r.b.recebidos.is_empty(), "a origem não pertence ao A");
    // Já uma origem válida passa.
    let bom = pacote(IP_A, IP_B, b"y");
    r.a.dev.send(&bom).unwrap();
    r.trocar_tudo(r.em(1.0));
    assert_eq!(r.b.recebidos, vec![bom]);
}

#[test]
fn um_datagrama_alterado_no_caminho_nao_chega() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"1")).unwrap();
    r.trocar_tudo(r.em(0.0));
    r.b.recebidos.clear();
    r.a.dev.send(&pacote(IP_A, IP_B, b"2")).unwrap();
    r.trocar(r.em(1.0), &mut |_, _| true);
    assert_eq!(r.b.recebidos.len(), 1);
    r.b.recebidos.clear();
    // Agora um datagrama de dados com um bit trocado.
    r.a.dev.send(&pacote(IP_A, IP_B, b"3")).unwrap();
    r.a.drenar(r.em(2.0));
    let (_, mut d) = r.a.saida.remove(0);
    assert_eq!(tipo(&d), 4);
    let n = d.len();
    d[n - 1] ^= 1;
    let origem = r.a.addr;
    r.b.receber(&d, origem, r.em(2.0));
    assert!(r.b.recebidos.is_empty());
}

#[test]
fn a_fila_de_espera_guarda_so_os_128_mais_novos() {
    let mut r = Rede::nova(None);
    let ps: Vec<_> = (0..200u8).map(|i| pacote(IP_A, IP_B, &[i; 8])).collect();
    for p in &ps {
        r.a.dev.send(p).unwrap();
    }
    r.trocar_tudo(r.em(0.0));
    assert_eq!(r.b.recebidos, ps[200 - 128..].to_vec());
}

#[test]
fn o_peer_removido_nao_recebe_mais() {
    let mut r = Rede::nova(None);
    r.a.dev.send(&pacote(IP_A, IP_B, b"x")).unwrap();
    r.trocar_tudo(r.em(0.0));
    let pub_a = r.a.dev.public_key();
    assert!(r.b.dev.remove_peer(&pub_a));
    r.b.recebidos.clear();
    r.a.dev.send(&pacote(IP_A, IP_B, b"y")).unwrap();
    r.trocar_tudo(r.em(1.0));
    assert!(r.b.recebidos.is_empty());
}

#[test]
fn iniciacoes_simultaneas_dos_dois_lados_chegam_a_uma_sessao() {
    let mut r = Rede::nova(None);
    let (pub_a, pub_b) = (r.a.dev.public_key(), r.b.dev.public_key());
    let addr_a = r.a.addr;
    r.b.dev.set_endpoint(&pub_a, Some(addr_a));
    r.a.dev.peer_mut(&pub_b).unwrap().connect();
    r.b.dev.peer_mut(&pub_a).unwrap().connect();
    r.trocar_tudo(r.em(0.0));
    let agora = r.em(0.0);
    assert!(
        r.a.dev.peer(&pub_b).unwrap().has_session(agora),
        "A sem sessão"
    );
    assert!(
        r.b.dev.peer(&pub_a).unwrap().has_session(agora),
        "B sem sessão"
    );
    // E os dados passam nos dois sentidos.
    let ida = pacote(IP_A, IP_B, b"ida");
    r.a.dev.send(&ida).unwrap();
    let volta = pacote(IP_B, IP_A, b"volta");
    r.b.dev.send(&volta).unwrap();
    r.trocar_tudo(r.em(1.0));
    assert!(r.b.recebidos.contains(&ida));
    assert!(r.a.recebidos.contains(&volta));
}
