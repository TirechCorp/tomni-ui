//! Os testes do `config.yml`: as fixtures, os avisos, as recusas e as conferências do `check`.

use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tomni_ui_nebula::{
    Cipher, Config, Severity, StaticMapNetwork, Version, Warning, parse_duration,
};

/// 2027-01-01, dentro da validade das fixtures.
const AGORA: i64 = 1_798_761_600;

fn pasta() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn texto(nome: &str) -> String {
    fs::read_to_string(pasta().join(nome)).unwrap()
}

fn ler(nome: &str) -> (Config, Vec<Warning>) {
    Config::from_yaml(&texto(nome), Some(&pasta())).unwrap()
}

fn ip(texto: &str) -> IpAddr {
    texto.parse().unwrap()
}

fn mensagens(config: &Config, agora: i64) -> Vec<(Severity, String)> {
    config
        .check(agora)
        .into_iter()
        .map(|problema| (problema.severity, problema.message))
        .collect()
}

#[test]
fn o_config_de_exemplo_do_nebula_e_lido_sem_erro() {
    let (config, avisos) = Config::from_yaml(&texto("config-exemplo.yml"), None).unwrap();
    // Sem a pasta, os três caminhos do pki ficam por ler, e cada um vira um aviso.
    let chaves: Vec<&str> = avisos.iter().map(|aviso| aviso.key.as_str()).collect();
    assert_eq!(chaves, ["pki.ca", "pki.cert", "pki.key"]);
    assert!(
        avisos[0].message.contains("/etc/nebula/ca.crt"),
        "{}",
        avisos[0]
    );
    assert_eq!(
        config.static_host_map[&ip("192.168.100.1")],
        ["100.64.22.11:4242"]
    );
    assert_eq!(config.lighthouse.hosts, [ip("192.168.100.1")]);
    assert_eq!(config.lighthouse.interval, Duration::from_secs(60));
    assert_eq!(config.listen.host, "::");
    assert_eq!(config.listen.port, 4242);
    assert!(config.punchy.punch);
    assert!(!config.punchy.respond);
    assert_eq!(config.punchy.delay, Duration::from_secs(1));
    assert_eq!(config.punchy.respond_delay, Duration::from_secs(5));
    assert!(config.relay.use_relays);
    assert_eq!(config.mtu, 1300);
    assert_eq!(config.cipher, Cipher::Aes);
    assert_eq!(config.handshakes.retries, 10);
    assert_eq!(config.handshakes.try_interval, Duration::from_millis(100));
    assert!(config.unsafe_routes.is_empty());
    assert!(config.pki.disconnect_invalid);
    // O firewall de saída do exemplo deixa tudo passar: nada a avisar sobre ele.
    assert!(avisos.iter().all(|aviso| aviso.key != "firewall.outbound"));
    // Com a pasta, os caminhos absolutos de `/etc/nebula` não existem: o erro diz qual.
    let erro = Config::from_yaml(&texto("config-exemplo.yml"), Some(&pasta())).unwrap_err();
    assert!(
        erro.to_string()
            .starts_with("pki.ca: não foi possível ler /etc/nebula/ca.crt"),
        "{erro}"
    );
}

#[test]
fn config_de_no_com_caminhos_relativos_e_opcoes() {
    let (config, avisos) = ler("config-no.yml");
    assert_eq!(config.static_host_map[&ip("10.42.0.1")].len(), 2);
    assert_eq!(config.static_map_network, StaticMapNetwork::Ip4);
    assert_eq!(config.lighthouse.interval, Duration::from_secs(30));
    assert_eq!(config.listen.port, 0);
    assert!(config.punchy.respond);
    assert_eq!(config.punchy.delay, Duration::from_millis(500));
    assert_eq!(config.cipher, Cipher::ChaChaPoly);
    assert_eq!(config.handshakes.retries, 5);
    assert_eq!(config.handshakes.try_interval, Duration::from_millis(200));
    assert_eq!(config.mtu, 1200);
    assert_eq!(config.unsafe_routes.len(), 1);
    assert_eq!(config.unsafe_routes[0].route.to_string(), "192.168.50.0/24");
    assert_eq!(config.unsafe_routes[0].via, [ip("10.42.0.25")]);
    assert_eq!(config.pki.blocklist.len(), 1);
    // Uma regra de saída restrita ainda não é aplicada.
    assert_eq!(avisos.len(), 1, "{avisos:?}");
    assert_eq!(avisos[0].key, "firewall.outbound");
    assert!(
        avisos[0]
            .message
            .contains("regras de saída ainda não aplicadas")
    );

    let cred = config.credentials().unwrap();
    assert_eq!(cred.initiating, Version::V2);
    assert!(cred.v1.is_none());
    assert_eq!(cred.default_cert().name(), "no-v2");
    assert_eq!(cred.address().unwrap().to_string(), "10.42.0.20/24");
    assert!(
        mensagens(&config, AGORA).is_empty(),
        "{:?}",
        mensagens(&config, AGORA)
    );
}

#[test]
fn o_pki_embutido_vale_como_o_por_caminho() {
    let indentar = |nome: &str| {
        texto(nome)
            .lines()
            .map(|linha| format!("    {linha}\n"))
            .collect::<String>()
    };
    let yaml = format!(
        "pki:\n  ca: |\n{}  cert: |\n{}  key: |\n{}static_host_map:\n  \"10.42.0.1\": [\"127.0.0.1:4242\"]\nlighthouse:\n  hosts: [\"10.42.0.1\"]\n",
        indentar("ca-25519.crt"),
        indentar("no-v2.crt"),
        indentar("no-v2.key")
    );
    let (config, avisos) = Config::from_yaml(&yaml, None).unwrap();
    assert!(avisos.is_empty(), "{avisos:?}");
    assert_eq!(config.credentials().unwrap().default_cert().name(), "no-v2");
    assert!(mensagens(&config, AGORA).is_empty());
    // O `Debug` do config não mostra a chave.
    assert!(!format!("{config:?}").contains("PRIVATE KEY"));
}

#[test]
fn set_pki_troca_o_pki_pelo_do_cofre() {
    let (mut config, _) = Config::from_yaml(&texto("config-exemplo.yml"), None).unwrap();
    assert!(
        config
            .credentials()
            .unwrap_err()
            .to_string()
            .contains("falta a CA")
    );
    config.set_pki(
        &texto("ca-25519.crt"),
        &texto("no-v1.crt"),
        &texto("no-v1.key"),
    );
    let cred = config.credentials().unwrap();
    assert_eq!(cred.initiating, Version::V1);
    assert_eq!(cred.address().unwrap().to_string(), "10.42.0.21/24");
    assert!(mensagens(&config, AGORA).is_empty());
}

#[test]
fn v1v2_inicia_pela_v1_e_a_versao_pode_ser_forcada() {
    let (mut config, _) = ler("config-no.yml");
    config.set_pki(
        &texto("ca-25519.crt"),
        &texto("no-v1v2.crt"),
        &texto("no-v1v2.key"),
    );
    let cred = config.credentials().unwrap();
    assert!(cred.v1.is_some() && cred.v2.is_some());
    assert_eq!(cred.initiating, Version::V1);
    config.pki.initiating_version = Some(2);
    assert_eq!(config.credentials().unwrap().initiating, Version::V2);
    // Só com a v2, pedir a v1 é erro.
    config.set_pki(
        &texto("ca-25519.crt"),
        &texto("no-v2.crt"),
        &texto("no-v2.key"),
    );
    config.pki.initiating_version = Some(1);
    let erro = config.credentials().unwrap_err();
    assert_eq!(
        erro.to_string(),
        "pki.initiating_version 1 exige um certificado v1 no pki.cert"
    );
    let erro = Config::from_yaml("pki:\n  initiating_version: 3\n", None).unwrap_err();
    assert!(
        erro.to_string()
            .contains("pki.initiating_version desconhecida: 3"),
        "{erro}"
    );
}

#[test]
fn o_check_diz_o_que_esta_errado_em_portugues() {
    let (mut config, _) = ler("config-no.yml");
    // Outra CA.
    config.set_pki(
        &texto("ca-outra.crt"),
        &texto("no-v2.crt"),
        &texto("no-v2.key"),
    );
    assert_eq!(
        mensagens(&config, AGORA),
        [(
            Severity::Error,
            "o certificado no-v2 não foi assinado por nenhuma CA do config".to_owned()
        )]
    );
    // A chave de outro nó.
    config.set_pki(
        &texto("ca-25519.crt"),
        &texto("no-v2.crt"),
        &texto("no-v2-b.key"),
    );
    assert_eq!(
        mensagens(&config, AGORA),
        [(
            Severity::Error,
            "a chave privada não corresponde ao certificado no-v2".to_owned()
        )]
    );
    // Vencido.
    config.set_pki(
        &texto("ca-25519.crt"),
        &texto("no-vencido.crt"),
        &texto("no-vencido.key"),
    );
    let problemas = mensagens(&config, AGORA);
    assert_eq!(problemas.len(), 1);
    assert!(
        problemas[0]
            .1
            .starts_with("o certificado no-vencido venceu em 22/10/2026"),
        "{problemas:?}"
    );
    // Na lista de bloqueio.
    config.set_pki(
        &texto("ca-25519.crt"),
        &texto("no-v2.crt"),
        &texto("no-v2.key"),
    );
    let impressao =
        tomni_ui_nebula::Certificate::from_pem(&texto("no-v2.crt")).unwrap()[0].fingerprint();
    config.pki.blocklist.push(impressao);
    assert_eq!(
        mensagens(&config, AGORA),
        [(
            Severity::Error,
            "o certificado no-v2 está na lista de bloqueio do config".to_owned()
        )]
    );
}

#[test]
fn certificado_perto_do_fim_avisa_com_a_data() {
    let (config, _) = ler("config-no.yml");
    let cert = &tomni_ui_nebula::Certificate::from_pem(&texto("no-v2.crt")).unwrap()[0];
    let problemas = config.check(cert.not_after() - 10 * 24 * 3600);
    assert_eq!(problemas.len(), 1);
    assert_eq!(problemas[0].severity, Severity::Warning);
    assert_eq!(
        problemas[0].message,
        "o certificado no-v2 vence em 28/09/2126"
    );
}

#[test]
fn sem_lighthouse_nem_static_host_map_e_erro() {
    let (mut config, _) = ler("config-no.yml");
    config.static_host_map.clear();
    config.lighthouse.hosts.clear();
    let problemas = mensagens(&config, AGORA);
    assert_eq!(problemas.len(), 1);
    assert!(
        problemas[0]
            .1
            .contains("não tem lighthouse nem static_host_map"),
        "{problemas:?}"
    );
    // Um lighthouse sem endereço também não leva a lugar nenhum.
    config.lighthouse.hosts.push(ip("10.42.0.1"));
    config
        .static_host_map
        .insert(ip("10.42.0.9"), vec!["1.2.3.4:4242".to_owned()]);
    let problemas = mensagens(&config, AGORA);
    assert!(
        problemas[0]
            .1
            .contains("o lighthouse 10.42.0.1 não está no static_host_map"),
        "{problemas:?}"
    );
}

#[test]
fn p256_e_recusada_com_a_mensagem_certa() {
    let (mut config, _) = ler("config-no.yml");
    config.set_pki(
        &texto("ca-p256.crt"),
        &texto("no-p256.crt"),
        &texto("no-p256.key"),
    );
    assert_eq!(
        mensagens(&config, AGORA),
        [(
            Severity::Error,
            "certificado P-256 ainda não é suportado".to_owned()
        )]
    );
}

#[test]
fn o_config_de_lighthouse_e_recusado() {
    let erro = Config::from_yaml(&texto("config-lighthouse.yml"), Some(&pasta())).unwrap_err();
    assert_eq!(
        erro.to_string(),
        "este config é de um lighthouse (lighthouse.am_lighthouse: true); use o config de um nó"
    );
}

#[test]
fn erros_de_valor_trazem_a_chave_e_o_motivo() {
    let erro = |yaml: &str| Config::from_yaml(yaml, None).unwrap_err().to_string();
    assert_eq!(
        erro("cipher: rc4\n"),
        "cifra desconhecida: rc4 (use aes ou chachapoly)"
    );
    assert!(erro("a: [1\n").contains("YAML"));
    assert!(erro("- a\n- b\n").contains("mapa de chaves"));
    assert!(erro("static_host_map:\n  nao-ip: [\"a:1\"]\n").contains("static_host_map: nao-ip"));
    assert!(erro("lighthouse:\n  hosts: [\"x\"]\n").contains("lighthouse.hosts: x"));
    assert!(erro("listen:\n  port: 70000\n").contains("listen.port inválida: 70000"));
    assert!(erro("static_map:\n  network: ipx\n").contains("static_map.network"));
    assert!(erro("tun:\n  unsafe_routes:\n    - route: 1.2.3.0/24\n").contains("não tem via"));
}

#[test]
fn chaves_desconhecidas_e_recursos_ausentes_viram_avisos() {
    let (_, avisos) = Config::from_yaml(
        "esquisita: 1\nsshd:\n  enabled: true\nstats:\n  type: prometheus\nrelay:\n  am_relay: true\nlighthouse:\n  serve_dns: true\npunchy:\n  delay: abc\n",
        None,
    )
    .unwrap();
    let chaves: Vec<&str> = avisos.iter().map(|aviso| aviso.key.as_str()).collect();
    assert_eq!(
        chaves,
        [
            "esquisita",
            "punchy.delay",
            "relay.am_relay",
            "sshd",
            "stats",
            "lighthouse.serve_dns"
        ]
    );
    assert!(avisos[1].message.contains("a duração abc não é válida"));
}

#[test]
fn as_chaves_antigas_do_punchy_continuam_valendo() {
    let (config, _) = Config::from_yaml("punchy: true\npunch_back: true\n", None).unwrap();
    assert!(config.punchy.punch && config.punchy.respond);
}

#[test]
fn duracoes_no_formato_do_nebula() {
    for (texto, esperado) in [
        ("1s", Some(Duration::from_secs(1))),
        ("500ms", Some(Duration::from_millis(500))),
        ("5m", Some(Duration::from_secs(300))),
        ("1h30m", Some(Duration::from_secs(5400))),
        ("1.5s", Some(Duration::from_millis(1500))),
        ("250us", Some(Duration::from_micros(250))),
        ("0", Some(Duration::ZERO)),
        (" 2s ", Some(Duration::from_secs(2))),
        ("", None),
        ("s", None),
        ("10", None),
        ("1d", None),
        ("-1s", None),
        ("1s2", None),
    ] {
        assert_eq!(parse_duration(texto), esperado, "{texto:?}");
    }
}

#[test]
fn caminho_absoluto_e_pasta_vazia() {
    // Um caminho absoluto ignora a pasta.
    let absoluto = pasta().join("ca-25519.crt");
    let yaml = format!("pki:\n  ca: {}\n", absoluto.display());
    let (config, _) = Config::from_yaml(&yaml, Some(Path::new("/nao/existe"))).unwrap();
    assert!(config.pki.ca.unwrap().contains("NEBULA CERTIFICATE"));
}
