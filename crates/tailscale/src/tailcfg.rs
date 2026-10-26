//! Os tipos JSON que o cliente e o plano de controle trocam (o `tailcfg` do protocolo).
//!
//! Só os campos que o Tomni UI usa. Os nomes seguem o JSON do protocolo (PascalCase, com
//! algumas exceções que cada campo marca). Campos que o servidor manda e que o crate não
//! conhece são ignorados sem erro; campos "zerados" que o protocolo omite não são escritos.

use std::net::SocketAddr;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::key::{ChallengePublic, DiscoPublic, LockPublic, NodePublic};

/// A versão de capacidade que o cliente declara ao controle.
///
/// É um número que cresce a cada mudança do protocolo; o controle decide o que pode mandar
/// com base nele. O 113 é o piso que o Headscale atual exige (a versão 0.29 recusa clientes
/// abaixo disso) e já traz tudo o que o crate usa: o mapa em fluxo com `zstd`, os deltas de
/// pares (`PeersChangedPatch`) e o DERP. Subir o número sem implementar o que ele promete
/// faz o controle mandar coisas que o cliente não entende, então só mexa nele junto com o
/// código.
pub const CAPABILITY_VERSION: u32 = 113;

/// O número de uma região do DERP.
pub type DerpRegionId = i32;

fn vazio(s: &str) -> bool {
    s.is_empty()
}

fn falso(b: &bool) -> bool {
    !*b
}

/// O instante "zero" do Go (`0001-01-01T00:00:00Z`), que o protocolo usa para "sem data".
pub fn tempo_zero() -> Timestamp {
    "0001-01-01T00:00:00Z"
        .parse()
        .expect("a data zero do protocolo é válida")
}

fn tempo_zero_padrao() -> Timestamp {
    tempo_zero()
}

/// Se a data é a "zero" do protocolo.
pub fn e_tempo_zero(t: &Timestamp) -> bool {
    *t == tempo_zero()
}

/// O que a máquina conta de si mesma ao controle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Hostinfo {
    /// A versão do cliente (no formato "nome/versão", dita com honestidade).
    #[serde(rename = "IPNVersion", skip_serializing_if = "vazio")]
    pub ipn_version: String,
    /// O sistema: `linux`, `windows`, `macOS`…
    #[serde(rename = "OS", skip_serializing_if = "vazio")]
    pub os: String,
    /// A versão do sistema.
    #[serde(rename = "OSVersion", skip_serializing_if = "vazio")]
    pub os_version: String,
    /// O nome da máquina.
    #[serde(skip_serializing_if = "vazio")]
    pub hostname: String,
    /// Se a máquina recusa conexões que chegam pela tailnet.
    #[serde(skip_serializing_if = "falso")]
    pub shields_up: bool,
    /// As sub-redes que a máquina oferece. O app nunca anuncia nada: fica sempre vazio.
    #[serde(
        rename = "RoutableIPs",
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lista_ou_nula"
    )]
    pub routable_ips: Vec<String>,
    /// As etiquetas de ACL que a máquina pede.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lista_ou_nula"
    )]
    pub request_tags: Vec<String>,
    /// O que o cliente sabe da rede.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_info: Option<NetInfo>,
    /// Se o cliente roda sem interface de rede (pilha própria no processo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub userspace: Option<bool>,
}

/// O que o cliente sabe da rede dele.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NetInfo {
    /// Se o UDP funciona.
    #[serde(rename = "WorkingUDP", skip_serializing_if = "Option::is_none")]
    pub working_udp: Option<bool>,
    /// Se o IPv6 funciona.
    #[serde(rename = "WorkingIPv6", skip_serializing_if = "Option::is_none")]
    pub working_ipv6: Option<bool>,
    /// A região do DERP de casa (0 = ainda não escolhida).
    #[serde(rename = "PreferredDERP", skip_serializing_if = "is_zero")]
    pub preferred_derp: DerpRegionId,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

/// A autenticação do registro: a chave de autenticação (`tskey-…` ou a do Headscale).
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RegisterAuth {
    /// A chave de autenticação.
    #[serde(skip_serializing_if = "vazio")]
    pub auth_key: String,
}

impl std::fmt::Debug for RegisterAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A chave de autenticação é segredo: não aparece em log nenhum.
        f.debug_struct("RegisterAuth")
            .field("auth_key", &"***")
            .finish()
    }
}

/// O pedido de registro (`POST /machine/register`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegisterRequest {
    /// A versão de capacidade do cliente.
    pub version: u32,
    /// A chave de nó (a do WireGuard) que se quer registrar.
    pub node_key: NodePublic,
    /// A chave de nó anterior, ao trocar de chave; zerada quando não há.
    pub old_node_key: NodePublic,
    /// A chave do bloqueio de rede; o crate manda zerada.
    #[serde(rename = "NLKey")]
    pub nl_key: LockPublic,
    /// A chave de autenticação, quando se tem uma.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<RegisterAuth>,
    /// Quando a chave de nó vence; a data zero quer dizer "o controle decide", e uma data no
    /// passado pede o logout.
    #[serde(default = "tempo_zero_padrao")]
    pub expiry: Timestamp,
    /// A URL de login do primeiro pedido, no segundo pedido do fluxo pelo navegador.
    pub followup: String,
    /// O que a máquina conta de si.
    pub hostinfo: Option<Hostinfo>,
    /// Se a máquina é efêmera (some do controle quando fica offline).
    #[serde(skip_serializing_if = "falso", default)]
    pub ephemeral: bool,
    /// A assinatura do Tailnet Lock; sempre vazia aqui.
    pub node_key_signature: Option<String>,
    /// Quando o pedido foi montado.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timestamp: Option<Timestamp>,
}

/// O usuário dono do nó.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct User {
    /// O número do usuário.
    #[serde(rename = "ID")]
    pub id: i64,
    /// O nome para mostrar.
    pub display_name: String,
}

/// O login do usuário.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Login {
    /// O provedor de identidade (`google`, `github`, `oidc`…).
    pub provider: String,
    /// O nome de login, em geral um e-mail.
    pub login_name: String,
    /// O nome para mostrar.
    pub display_name: String,
}

/// A resposta do registro.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RegisterResponse {
    /// O dono.
    pub user: User,
    /// O login.
    pub login: Login,
    /// Se a chave de nó precisa ser trocada por outra.
    pub node_key_expired: bool,
    /// Se a máquina já está autorizada a entrar na tailnet.
    pub machine_authorized: bool,
    /// Se não vazia, a autorização está pendente: o usuário abre esta URL.
    #[serde(rename = "AuthURL")]
    pub auth_url: String,
    /// A assinatura do Tailnet Lock, quando a tailnet o usa.
    pub node_key_signature: Option<String>,
    /// O motivo de uma recusa.
    pub error: String,
}

/// De onde veio um endereço de endpoint anunciado ao controle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EndpointType(pub u8);

impl EndpointType {
    /// Desconhecido.
    pub const UNKNOWN: Self = Self(0);
    /// Um endereço da própria máquina.
    pub const LOCAL: Self = Self(1);
    /// Descoberto por STUN.
    pub const STUN: Self = Self(2);
    /// Aberto por mapeamento de porta (UPnP, NAT-PMP).
    pub const PORTMAPPED: Self = Self(3);
    /// STUN, com a porta local igual à externa.
    pub const STUN4_LOCAL_PORT: Self = Self(4);
    /// Configurado à mão.
    pub const EXPLICIT_CONF: Self = Self(5);
}

/// O pedido do mapa (`POST /machine/map`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MapRequest {
    /// A versão de capacidade do cliente.
    pub version: u32,
    /// `"zstd"` para receber o mapa comprimido; vazio para sem compressão.
    #[serde(skip_serializing_if = "vazio", default)]
    pub compress: String,
    /// Se o servidor deve mandar sinais de vida.
    #[serde(skip_serializing_if = "falso", default)]
    pub keep_alive: bool,
    /// A chave de nó.
    pub node_key: NodePublic,
    /// A chave de disco.
    pub disco_key: DiscoPublic,
    /// Se a resposta é um fluxo que não termina.
    #[serde(skip_serializing_if = "falso", default)]
    pub stream: bool,
    /// O que a máquina conta de si.
    pub hostinfo: Option<Hostinfo>,
    /// Onde alcançar esta máquina.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub endpoints: Vec<SocketAddr>,
    /// De onde veio cada endpoint (na mesma ordem).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub endpoint_types: Vec<EndpointType>,
    /// Se o servidor deve omitir a lista de pares.
    #[serde(skip_serializing_if = "falso", default)]
    pub omit_peers: bool,
    /// Se é só uma atualização, sem resposta.
    #[serde(skip_serializing_if = "falso", default)]
    pub read_only: bool,
}

/// A carga antecipada que o servidor pode mandar logo depois do handshake Noise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EarlyNoise {
    /// A chave pública de desafio da conexão.
    #[serde(rename = "nodeKeyChallenge")]
    pub node_key_challenge: ChallengePublic,
}

/// Lê uma chave que o servidor pode mandar vazia, nula ou ausente (vira a chave zero).
fn chave_leniente<'de, D, K>(d: D) -> Result<K, D::Error>
where
    D: serde::Deserializer<'de>,
    K: std::str::FromStr + Default,
    K::Err: std::fmt::Display,
{
    let texto = <Option<std::borrow::Cow<'de, str>>>::deserialize(d)?;
    match texto.as_deref() {
        None | Some("") => Ok(K::default()),
        Some(t) => t.parse().map_err(serde::de::Error::custom),
    }
}

/// Lê uma data em que o "zero" do Go (ou nulo, ou ausente) quer dizer "sem data".
fn tempo_opcional<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Timestamp>, D::Error> {
    let t = <Option<Timestamp>>::deserialize(d)?;
    Ok(t.filter(|t| !e_tempo_zero(t)))
}

/// Lê uma lista de endereços de rede, descartando o que não for um `ip:porta` válido.
fn enderecos_lenientes<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<SocketAddr>, D::Error> {
    let textos = <Option<Vec<String>>>::deserialize(d)?.unwrap_or_default();
    Ok(textos.iter().filter_map(|t| t.parse().ok()).collect())
}

/// Lê uma lista que o Go escreve como `null` quando está vazia.
fn lista_ou_nula<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(<Option<Vec<T>>>::deserialize(d)?.unwrap_or_default())
}

/// Um nó da tailnet: o nosso ou um par.
///
/// Só os campos que o crate usa. Os endereços (`addresses`, `allowed_ips`, `primary_routes`)
/// ficam como o protocolo os escreve (`"100.64.0.1/32"`); o `NetMap` os interpreta.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Node {
    /// O número do nó no controle.
    #[serde(rename = "ID")]
    pub id: i64,
    /// O número estável (não muda ao trocar de chave).
    #[serde(rename = "StableID")]
    pub stable_id: String,
    /// O nome completo (FQDN, com o ponto final): `maquina.minha-tailnet.ts.net.`.
    pub name: String,
    /// O número do usuário dono.
    pub user: i64,
    /// A chave de nó (a do WireGuard).
    #[serde(deserialize_with = "chave_leniente")]
    pub key: NodePublic,
    /// Quando a chave de nó vence.
    #[serde(deserialize_with = "tempo_opcional")]
    pub key_expiry: Option<Timestamp>,
    /// A chave de disco, para o descobrimento de caminho.
    #[serde(deserialize_with = "chave_leniente")]
    pub disco_key: DiscoPublic,
    /// Os endereços da tailnet (`100.x.y.z/32`).
    #[serde(deserialize_with = "lista_ou_nula")]
    pub addresses: Vec<String>,
    /// Os destinos que se pode mandar a este nó (os endereços mais as sub-redes aprovadas).
    #[serde(rename = "AllowedIPs", deserialize_with = "lista_ou_nula")]
    pub allowed_ips: Vec<String>,
    /// As sub-redes que este nó anuncia e o controle aprovou.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub primary_routes: Vec<String>,
    /// Onde alcançar o nó direto.
    #[serde(deserialize_with = "enderecos_lenientes")]
    pub endpoints: Vec<SocketAddr>,
    /// A região do DERP de casa do nó (0 = nenhuma).
    #[serde(rename = "HomeDERP")]
    pub home_derp: DerpRegionId,
    /// O que o nó conta de si.
    pub hostinfo: Option<Hostinfo>,
    /// Quando o nó foi criado.
    #[serde(deserialize_with = "tempo_opcional")]
    pub created: Option<Timestamp>,
    /// As etiquetas de ACL do nó.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub tags: Vec<String>,
    /// Quando o controle viu o nó por último (só para o nó offline).
    #[serde(deserialize_with = "tempo_opcional")]
    pub last_seen: Option<Timestamp>,
    /// Se o nó está conectado ao controle (ausente = o controle não sabe).
    pub online: Option<bool>,
    /// Se a máquina está autorizada na tailnet.
    pub machine_authorized: bool,
    /// Se a chave do nó venceu.
    pub expired: bool,
}

/// Uma mudança parcial de um par (`PeersChangedPatch`): só os campos presentes mudaram.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PeerChange {
    /// O número do nó que mudou.
    #[serde(rename = "NodeID")]
    pub node_id: i64,
    /// A nova região do DERP de casa.
    #[serde(rename = "DERPRegion")]
    pub derp_region: Option<DerpRegionId>,
    /// Os endpoints novos (a lista inteira, não um acréscimo).
    pub endpoints: Option<Vec<String>>,
    /// A chave de nó nova.
    pub key: Option<NodePublic>,
    /// A chave de disco nova.
    pub disco_key: Option<DiscoPublic>,
    /// Se o nó está conectado.
    pub online: Option<bool>,
    /// Quando o controle viu o nó por último.
    pub last_seen: Option<Timestamp>,
    /// Quando a chave de nó vence.
    pub key_expiry: Option<Timestamp>,
}

/// Um servidor DERP de uma região.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DerpNode {
    /// O nome do servidor (`1a`, `1b`…).
    pub name: String,
    /// A região a que pertence.
    #[serde(rename = "RegionID")]
    pub region_id: DerpRegionId,
    /// O nome DNS do servidor.
    pub host_name: String,
    /// O nome esperado no certificado (vazio = o `HostName`).
    pub cert_name: String,
    /// O endereço IPv4 fixo (vazio = resolver o `HostName`; `"none"` = sem IPv4).
    #[serde(rename = "IPv4")]
    pub ipv4: String,
    /// O endereço IPv6 fixo (vazio = resolver o `HostName`; `"none"` = sem IPv6).
    #[serde(rename = "IPv6")]
    pub ipv6: String,
    /// A porta do STUN (0 = 3478; -1 = sem STUN).
    #[serde(rename = "STUNPort")]
    pub stun_port: i32,
    /// Se o servidor só atende STUN, sem DERP.
    #[serde(rename = "STUNOnly")]
    pub stun_only: bool,
    /// A porta do DERP (0 = 443).
    #[serde(rename = "DERPPort")]
    pub derp_port: i32,
    /// Se o TLS não precisa ser válido (só para testes).
    pub insecure_for_tests: bool,
    /// Se o servidor atende também HTTP na porta 80.
    pub can_port80: bool,
}

/// Uma região do DERP.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DerpRegion {
    /// O número da região.
    #[serde(rename = "RegionID")]
    pub region_id: DerpRegionId,
    /// O código curto (`sao`, `nyc`…).
    pub region_code: String,
    /// O nome para mostrar (`São Paulo`).
    pub region_name: String,
    /// Se os clientes devem evitar a região.
    pub avoid: bool,
    /// Se a região não entra na medição de latência nem vira casa.
    pub no_measure_no_home: bool,
    /// Os servidores, em ordem de preferência.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub nodes: Vec<DerpNode>,
}

/// O mapa das regiões do DERP.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DerpMap {
    /// As regiões, pelo número.
    pub regions: std::collections::BTreeMap<DerpRegionId, Option<DerpRegion>>,
    /// Se as regiões padrão do Tailscale ficam de fora (o Headscale manda `true` quando
    /// tem o DERP dele só).
    pub omit_default_regions: bool,
}

/// Um servidor de nomes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Resolver {
    /// O endereço: um IP, `ip:porta` ou uma URL `https://` (DoH).
    pub addr: String,
}

/// Um registro de DNS que o controle manda a mais.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DnsRecord {
    /// O nome completo.
    pub name: String,
    /// O tipo (vazio = `A` ou `AAAA`, conforme o valor).
    #[serde(rename = "Type")]
    pub tipo: String,
    /// O valor (um IP).
    pub value: String,
}

/// A configuração de DNS da tailnet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DnsConfig {
    /// Os servidores de nomes globais.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub resolvers: Vec<Resolver>,
    /// Os servidores de um domínio (a chave é o sufixo).
    pub routes: std::collections::BTreeMap<String, Option<Vec<Resolver>>>,
    /// Os servidores que valem se não houver outros.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub fallback_resolvers: Vec<Resolver>,
    /// Os domínios de busca: o primeiro é o da tailnet.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub domains: Vec<String>,
    /// Se o MagicDNS está ligado (os nomes dos nós resolvem).
    pub proxied: bool,
    /// Os domínios para os quais o controle emite certificado.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub cert_domains: Vec<String>,
    /// Os registros a mais.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub extra_records: Vec<DnsRecord>,
}

/// O perfil de um usuário da tailnet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserProfile {
    /// O número do usuário.
    #[serde(rename = "ID")]
    pub id: i64,
    /// O nome de login, em geral um e-mail.
    pub login_name: String,
    /// O nome para mostrar.
    pub display_name: String,
}

/// Uma resposta do mapa (`POST /machine/map`): o mapa inteiro, um delta ou um sinal de vida.
///
/// Só os campos que o crate usa; o resto (`PacketFilter`, `SSHPolicy`…) é ignorado sem erro.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MapResponse {
    /// Só um sinal de vida: não traz mais nada.
    pub keep_alive: bool,
    /// O nosso nó (ausente quando nada mudou nele).
    pub node: Option<Node>,
    /// Quando não vazia, a lista completa de pares: substitui a que se tinha.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub peers: Vec<Node>,
    /// Pares novos ou alterados por inteiro.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub peers_changed: Vec<Node>,
    /// Os números dos pares que saíram.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub peers_removed: Vec<i64>,
    /// Mudanças parciais de pares.
    #[serde(rename = "PeersChangedPatch", deserialize_with = "lista_ou_nula")]
    pub peers_changed_patch: Vec<PeerChange>,
    /// Mudanças só de "visto por último" (a chave é a chave de nó).
    pub peer_seen_change: Option<std::collections::BTreeMap<String, bool>>,
    /// Mudanças só de "conectado" (a chave é o número do nó).
    pub online_change: Option<std::collections::BTreeMap<i64, bool>>,
    /// O mapa do DERP (ausente = não mudou).
    #[serde(rename = "DERPMap")]
    pub derp_map: Option<DerpMap>,
    /// A configuração de DNS (ausente = não mudou).
    #[serde(rename = "DNSConfig")]
    pub dns_config: Option<DnsConfig>,
    /// O nome da tailnet (vazio = não mudou).
    pub domain: String,
    /// Os perfis dos usuários (só os que mudaram).
    #[serde(deserialize_with = "lista_ou_nula")]
    pub user_profiles: Vec<UserProfile>,
    /// A hora do controle, para mostrar.
    #[serde(deserialize_with = "tempo_opcional")]
    pub control_time: Option<Timestamp>,
    /// Avisos de saúde que o controle manda.
    #[serde(deserialize_with = "lista_ou_nula")]
    pub health: Vec<String>,
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::key::NodeKey;

    const REGISTRO_AUTH_URL: &str = include_str!("../tests/amostras/register_authurl.json");
    const REGISTRO_AUTORIZADO: &str = include_str!("../tests/amostras/register_autorizado.json");
    const REGISTRO_ERRO: &str = include_str!("../tests/amostras/register_erro.json");
    const PEDIDO_COM_CHAVE: &str = include_str!("../tests/amostras/register_pedido_chave.json");
    const MAPA_COMPLETO: &str = include_str!("../tests/amostras/map_completo.json");
    const MAPA_DELTA: &str = include_str!("../tests/amostras/map_delta.json");
    const MAPA_HEADSCALE: &str = include_str!("../tests/amostras/map_headscale.json");

    #[test]
    fn a_data_zero_do_go_vai_e_volta() {
        assert!(e_tempo_zero(&tempo_zero()));
        assert_eq!(tempo_zero().to_string(), "0001-01-01T00:00:00Z");
    }

    #[test]
    fn le_a_resposta_com_url_de_login() {
        let r: RegisterResponse = serde_json::from_str(REGISTRO_AUTH_URL).unwrap();
        assert_eq!(r.auth_url, "https://login.exemplo.com/a/abc123");
        assert!(!r.machine_authorized);
        assert!(!r.node_key_expired);
        assert!(r.error.is_empty());
    }

    #[test]
    fn le_a_resposta_autorizada_ignorando_campos_novos() {
        let r: RegisterResponse = serde_json::from_str(REGISTRO_AUTORIZADO).unwrap();
        assert!(r.machine_authorized);
        assert_eq!(r.user.id, 7);
        assert_eq!(r.login.login_name, "ana@exemplo.com");
        assert_eq!(r.login.display_name, "Ana");
        assert!(r.auth_url.is_empty());
    }

    #[test]
    fn le_a_resposta_de_erro() {
        let r: RegisterResponse = serde_json::from_str(REGISTRO_ERRO).unwrap();
        assert_eq!(r.error, "chave de autenticação expirada");
    }

    #[test]
    fn o_pedido_tem_a_forma_do_protocolo() {
        let esperado: serde_json::Value = serde_json::from_str(PEDIDO_COM_CHAVE).unwrap();
        let no = NodeKey::from_bytes([1; 32]).public();
        let pedido = RegisterRequest {
            version: CAPABILITY_VERSION,
            node_key: no,
            old_node_key: NodePublic::default(),
            nl_key: LockPublic::default(),
            auth: Some(RegisterAuth {
                auth_key: "tskey-auth-teste".into(),
            }),
            expiry: tempo_zero(),
            followup: String::new(),
            hostinfo: Some(Hostinfo {
                ipn_version: "tirech-omni/0.1".into(),
                os: "linux".into(),
                hostname: "tirech".into(),
                shields_up: true,
                ..Hostinfo::default()
            }),
            ephemeral: false,
            node_key_signature: None,
            timestamp: None,
        };
        let mut json = serde_json::to_value(&pedido).unwrap();
        // A chave de nó muda a cada execução do teste de outra máquina; aqui é fixa.
        assert_eq!(json["NodeKey"], serde_json::json!(no.to_string()));
        json["NodeKey"] = esperado["NodeKey"].clone();
        assert_eq!(json, esperado);
    }

    #[test]
    fn o_hostinfo_nao_escreve_o_que_e_vazio() {
        let h = Hostinfo {
            hostname: "a".into(),
            shields_up: true,
            ..Hostinfo::default()
        };
        let json = serde_json::to_value(&h).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"Hostname": "a", "ShieldsUp": true})
        );
        assert!(json.get("RoutableIPs").is_none());
    }

    #[test]
    fn a_chave_de_autenticacao_nao_aparece_no_debug() {
        let a = RegisterAuth {
            auth_key: "tskey-auth-segredo".into(),
        };
        assert!(!format!("{a:?}").contains("segredo"));
    }

    #[test]
    fn o_pedido_do_mapa_so_escreve_o_que_foi_pedido() {
        let m = MapRequest {
            version: CAPABILITY_VERSION,
            compress: "zstd".into(),
            keep_alive: true,
            node_key: NodePublic::default(),
            disco_key: DiscoPublic::default(),
            stream: true,
            hostinfo: None,
            endpoints: vec!["192.0.2.1:41641".parse().unwrap()],
            endpoint_types: vec![EndpointType::STUN],
            omit_peers: false,
            read_only: false,
        };
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["Compress"], "zstd");
        assert_eq!(json["Endpoints"], serde_json::json!(["192.0.2.1:41641"]));
        assert_eq!(json["EndpointTypes"], serde_json::json!([2]));
        assert!(json.get("OmitPeers").is_none());
        assert!(json["Hostinfo"].is_null());
    }

    #[test]
    fn a_carga_antecipada_le() {
        let c = ChallengePublic::from_bytes([9; 32]);
        let texto = format!("{{\"nodeKeyChallenge\":\"{c}\"}}");
        let e: EarlyNoise = serde_json::from_str(&texto).unwrap();
        assert_eq!(e.node_key_challenge, c);
    }

    #[test]
    fn le_o_mapa_completo_do_tailscale() {
        let m: MapResponse = serde_json::from_str(MAPA_COMPLETO).unwrap();
        assert!(!m.keep_alive);
        let eu = m.node.as_ref().unwrap();
        assert_eq!(eu.id, 101);
        assert_eq!(eu.name, "tirech-omni.exemplo.ts.net.");
        assert_eq!(eu.addresses, ["100.64.0.1/32", "fd7a:115c:a1e0::1/128"]);
        assert_eq!(eu.key, NodePublic::from_bytes([1; 32]));
        assert_eq!(eu.key_expiry.unwrap().to_string(), "2027-03-01T12:00:00Z");
        assert!(eu.endpoints.is_empty());
        assert!(eu.tags.is_empty());
        assert_eq!(eu.online, Some(true));
        assert!(eu.hostinfo.as_ref().unwrap().shields_up);

        assert_eq!(m.peers.len(), 2);
        let servidor = &m.peers[0];
        assert_eq!(servidor.name, "servidor.exemplo.ts.net.");
        // A data zero do Go vira "sem data".
        assert_eq!(servidor.key_expiry, None);
        assert_eq!(servidor.primary_routes, ["192.168.50.0/24"]);
        assert_eq!(servidor.endpoints.len(), 3);
        assert_eq!(servidor.home_derp, 1);
        assert_eq!(servidor.online, Some(false));
        assert_eq!(
            servidor.last_seen.unwrap().to_string(),
            "2026-10-21T22:15:03.5Z"
        );
        assert_eq!(servidor.tags, ["tag:servidor"]);
        let h = servidor.hostinfo.as_ref().unwrap();
        assert_eq!(h.hostname, "Servidor");
        assert_eq!(h.routable_ips, ["192.168.50.0/24"]);
        // Um par antigo, sem o que o protocolo acrescentou depois.
        let notebook = &m.peers[1];
        assert_eq!(notebook.online, Some(true));
        assert_eq!(notebook.last_seen, None);
        assert!(notebook.primary_routes.is_empty());

        assert_eq!(m.domain, "exemplo.ts.net");
        assert_eq!(m.user_profiles.len(), 2);
        assert_eq!(m.user_profiles[0].login_name, "ana@exemplo.com");
        assert_eq!(
            m.control_time.unwrap().to_string(),
            "2026-10-22T10:00:00.123456789Z"
        );
    }

    #[test]
    fn le_o_mapa_do_derp_e_o_dns() {
        let m: MapResponse = serde_json::from_str(MAPA_COMPLETO).unwrap();
        let derp = m.derp_map.unwrap();
        assert!(derp.omit_default_regions);
        assert_eq!(derp.regions.len(), 3);
        // Uma região nula (o controle "apaga" uma região padrão) é lida como ausente.
        assert!(derp.regions[&2].is_none());
        let sao = derp.regions[&1].as_ref().unwrap();
        assert_eq!(sao.region_name, "São Paulo");
        assert_eq!(sao.nodes.len(), 2);
        assert_eq!(sao.nodes[0].host_name, "derp1.exemplo.com");
        assert_eq!(sao.nodes[0].ipv4, "203.0.113.10");
        assert_eq!(sao.nodes[0].ipv6, "none");
        assert!(sao.nodes[0].can_port80);
        assert_eq!(sao.nodes[1].stun_port, -1);
        let teste = derp.regions[&900].as_ref().unwrap();
        assert!(teste.avoid && teste.no_measure_no_home);
        assert!(teste.nodes[0].insecure_for_tests && teste.nodes[0].stun_only);
        assert_eq!(teste.nodes[0].stun_port, 3479);

        let dns = m.dns_config.unwrap();
        assert!(dns.proxied);
        assert_eq!(dns.resolvers[0].addr, "1.1.1.1");
        assert_eq!(
            dns.routes["empresa.interno"].as_ref().unwrap()[0].addr,
            "10.0.0.53"
        );
        assert!(dns.routes["ts.net"].is_none());
        assert_eq!(
            dns.fallback_resolvers[0].addr,
            "https://dns.exemplo.com/dns-query"
        );
        assert_eq!(dns.domains, ["exemplo.ts.net"]);
        assert_eq!(dns.cert_domains, ["tirech-omni.exemplo.ts.net"]);
        assert_eq!(dns.extra_records[0].name, "grafana.exemplo.ts.net");
        assert_eq!(dns.extra_records[0].value, "100.64.0.50");
    }

    #[test]
    fn le_os_deltas() {
        let m: MapResponse = serde_json::from_str(MAPA_DELTA).unwrap();
        assert!(m.peers.is_empty());
        assert!(m.node.is_none() && m.derp_map.is_none() && m.dns_config.is_none());
        assert_eq!(m.peers_changed.len(), 1);
        // Uma chave de disco ou de máquina que o servidor não mandou fica zerada.
        assert!(m.peers_changed[0].key_expiry.is_none());
        assert_eq!(m.peers_removed, [103]);
        let p = &m.peers_changed_patch[0];
        assert_eq!(p.node_id, 102);
        assert_eq!(p.derp_region, Some(1));
        assert_eq!(
            p.endpoints.as_deref(),
            Some(&["198.51.100.7:41700".to_string()][..])
        );
        assert_eq!(p.online, Some(true));
        assert!(p.key.is_none() && p.disco_key.is_none());
        assert!(m.online_change.unwrap()[&102]);
        assert_eq!(m.peer_seen_change.unwrap().len(), 1);
    }

    #[test]
    fn le_o_mapa_enxuto_do_headscale() {
        let m: MapResponse = serde_json::from_str(MAPA_HEADSCALE).unwrap();
        let eu = m.node.unwrap();
        assert_eq!(eu.online, None);
        assert_eq!(eu.last_seen, None);
        assert_eq!(eu.key_expiry, None);
        assert!(eu.endpoints.is_empty());
        assert_eq!(eu.home_derp, 999);
        assert!(m.peers.is_empty());
        assert!(!m.dns_config.unwrap().proxied);
        assert_eq!(m.derp_map.unwrap().regions.len(), 1);
    }

    #[test]
    fn um_sinal_de_vida_e_uma_resposta_vazia() {
        let m: MapResponse = serde_json::from_str(r#"{"KeepAlive":true}"#).unwrap();
        assert!(m.keep_alive);
        assert!(m.node.is_none() && m.peers.is_empty());
        let m: MapResponse = serde_json::from_str("{}").unwrap();
        assert!(!m.keep_alive);
    }

    #[test]
    fn chaves_vazias_ou_nulas_viram_zero_mas_lixo_e_erro() {
        let n: Node = serde_json::from_str(r#"{"ID":1,"Key":"","DiscoKey":null}"#).unwrap();
        assert!(n.key.is_zero() && n.disco_key.is_zero());
        assert!(serde_json::from_str::<Node>(r#"{"Key":"nodekey:123"}"#).is_err());
        assert!(serde_json::from_str::<Node>(r#"{"Key":"mkey:00"}"#).is_err());
    }

    #[test]
    fn o_mapa_vai_e_volta_pelo_json() {
        let m: MapResponse = serde_json::from_str(MAPA_COMPLETO).unwrap();
        let json = serde_json::to_string(&m).unwrap();
        let de_novo: MapResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(m, de_novo);
    }
}
