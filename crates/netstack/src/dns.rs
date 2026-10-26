//! O DNS dentro do túnel: consultas A sobre TCP (porta 53) aos servidores que a VPN empurrou,
//! com a lista de domínios de busca e um cache pelo TTL. Serve de [`Resolver`] da ponte SOCKS.
//!
//! Só IPv4 (a pilha ainda não fala IPv6, então o AAAA não serve de nada). O formato das
//! mensagens é o da RFC 1035, escrito aqui, com a compressão de nomes na leitura.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

use crate::NetStack;
use crate::socks::{Answer, Resolver};

/// Quanto cada servidor tem para responder uma consulta.
const QUERY_TIMEOUT: Duration = Duration::from_secs(4);
/// O menor e o maior tempo que uma resposta fica no cache, qualquer que seja o TTL do servidor.
const MIN_TTL: Duration = Duration::from_secs(5);
const MAX_TTL: Duration = Duration::from_secs(300);
/// O máximo de nomes no cache.
const CACHE_LIMIT: usize = 256;

/// Que nomes vão para o DNS da VPN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsScope {
    /// Todos: com `redirect-gateway`, o computador não pergunta nada fora do túnel.
    All,
    /// Os nomes sem ponto e os que terminam num destes domínios (os que a VPN empurrou). Os
    /// demais seguem para o resolvedor do computador.
    Domains(Vec<String>),
}

/// O resolvedor de DNS por TCP dentro do túnel.
pub struct DnsOverTcp {
    servers: Vec<Ipv4Addr>,
    search: Vec<String>,
    scope: DnsScope,
    cache: Mutex<HashMap<String, (Instant, Vec<IpAddr>)>>,
}

impl DnsOverTcp {
    /// `servers` na ordem em que a VPN empurrou, `search` os domínios de busca (para nomes sem
    /// ponto) e `scope` quais nomes são do DNS da VPN.
    pub fn new(servers: Vec<Ipv4Addr>, search: Vec<String>, scope: DnsScope) -> DnsOverTcp {
        let clean = |list: Vec<String>| -> Vec<String> {
            list.into_iter()
                .map(|d| d.trim().trim_matches('.').to_ascii_lowercase())
                .filter(|d| !d.is_empty())
                .collect()
        };
        let scope = match scope {
            DnsScope::All => DnsScope::All,
            DnsScope::Domains(list) => DnsScope::Domains(clean(list)),
        };
        DnsOverTcp {
            servers,
            search: clean(search),
            scope,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn handles(&self, name: &str) -> bool {
        match &self.scope {
            DnsScope::All => true,
            DnsScope::Domains(domains) => {
                !name.contains('.')
                    || domains.iter().chain(&self.search).any(|domain| {
                        name == domain
                            || name
                                .strip_suffix(domain.as_str())
                                .is_some_and(|rest| rest.ends_with('.'))
                    })
            }
        }
    }

    /// Os nomes completos a perguntar, na ordem: os com domínio de busca antes, para quem digitou
    /// só `servidor`; depois o nome puro.
    fn candidates(&self, name: &str) -> Vec<String> {
        let mut list = Vec::new();
        if !name.contains('.') {
            for domain in &self.search {
                list.push(format!("{name}.{domain}"));
            }
        }
        list.push(name.to_owned());
        list
    }

    fn cached(&self, name: &str) -> Option<Vec<IpAddr>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        match cache.get(name) {
            Some((until, list)) if *until > Instant::now() => Some(list.clone()),
            Some(_) => {
                cache.remove(name);
                None
            }
            None => None,
        }
    }

    fn remember(&self, name: &str, list: &[IpAddr], ttl: Duration) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if cache.len() >= CACHE_LIMIT {
            cache.retain(|_, (until, _)| *until > now);
        }
        if cache.len() < CACHE_LIMIT {
            cache.insert(
                name.to_owned(),
                (now + ttl.clamp(MIN_TTL, MAX_TTL), list.to_vec()),
            );
        }
    }

    async fn lookup(&self, stack: &NetStack, name: &str) -> Answer {
        if !self.handles(name) {
            return Answer::Skip;
        }
        for full in self.candidates(name) {
            if let Some(list) = self.cached(&full) {
                return Answer::Found(list);
            }
            for server in &self.servers {
                match ask(stack, *server, &full).await {
                    Ok(Reply::Found(list, ttl)) => {
                        self.remember(&full, &list, ttl);
                        return Answer::Found(list);
                    }
                    // O servidor respondeu que o nome não existe (ou não tem endereço): o
                    // próximo servidor diria o mesmo, vale o próximo nome.
                    Ok(Reply::Empty) => break,
                    Err(_) => continue,
                }
            }
        }
        Answer::NotFound
    }
}

impl Resolver for DnsOverTcp {
    fn resolve<'a>(
        &'a self,
        stack: &'a NetStack,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send + 'a>> {
        Box::pin(async move {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            self.lookup(stack, &name).await
        })
    }
}

enum Reply {
    Found(Vec<IpAddr>, Duration),
    Empty,
}

async fn ask(stack: &NetStack, server: Ipv4Addr, name: &str) -> io::Result<Reply> {
    let mut id = [0u8; 2];
    getrandom::fill(&mut id).map_err(|e| io::Error::other(e.to_string()))?;
    let id = u16::from_be_bytes(id);
    let query = build_query(id, name)?;
    let exchange = async {
        let mut tcp = stack
            .connect_timeout(SocketAddr::new(IpAddr::V4(server), 53), QUERY_TIMEOUT)
            .await?;
        tcp.write_all(&query).await?;
        let mut length = [0u8; 2];
        tcp.read_exact(&mut length).await?;
        let mut answer = vec![0u8; usize::from(u16::from_be_bytes(length))];
        tcp.read_exact(&mut answer).await?;
        let _ = tcp.shutdown().await;
        io::Result::Ok(answer)
    };
    let answer = timeout(QUERY_TIMEOUT, exchange)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "o DNS da rede não respondeu"))??;
    parse_reply(id, &answer)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// A consulta A de `name`, já com o prefixo de 2 bytes do DNS sobre TCP.
fn build_query(id: u16, name: &str) -> io::Result<Vec<u8>> {
    let mut message = Vec::with_capacity(32 + name.len());
    message.extend_from_slice(&id.to_be_bytes());
    message.extend_from_slice(&[0x01, 0x00]); // RD
    message.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // QDCOUNT = 1
    if name.len() > 253 {
        return Err(invalid("nome longo demais para o DNS"));
    }
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(invalid("nome inválido para o DNS"));
        }
        message.push(label.len() as u8);
        message.extend_from_slice(label.as_bytes());
    }
    message.push(0);
    message.extend_from_slice(&[0, 1, 0, 1]); // A, IN
    let mut framed = Vec::with_capacity(message.len() + 2);
    framed.extend_from_slice(&(message.len() as u16).to_be_bytes());
    framed.extend_from_slice(&message);
    Ok(framed)
}

/// Pula um nome (com ou sem ponteiro de compressão) a partir de `pos`.
fn skip_name(message: &[u8], mut pos: usize) -> io::Result<usize> {
    loop {
        let length = *message
            .get(pos)
            .ok_or_else(|| invalid("resposta DNS cortada"))?;
        match length {
            0 => return Ok(pos + 1),
            l if l & 0xC0 == 0xC0 => return Ok(pos + 2),
            l if l & 0xC0 == 0 => pos += 1 + usize::from(l),
            _ => return Err(invalid("rótulo DNS desconhecido")),
        }
    }
}

fn parse_reply(id: u16, message: &[u8]) -> io::Result<Reply> {
    if message.len() < 12 {
        return Err(invalid("resposta DNS curta demais"));
    }
    if u16::from_be_bytes([message[0], message[1]]) != id {
        return Err(invalid("a resposta DNS não é da consulta"));
    }
    let flags = u16::from_be_bytes([message[2], message[3]]);
    if flags & 0x8000 == 0 {
        return Err(invalid("a resposta DNS não é uma resposta"));
    }
    match flags & 0x000F {
        0 => {}
        3 => return Ok(Reply::Empty), // NXDOMAIN
        rcode => {
            return Err(invalid(&format!(
                "o servidor DNS respondeu com o erro {rcode}"
            )));
        }
    }
    let questions = usize::from(u16::from_be_bytes([message[4], message[5]]));
    let answers = usize::from(u16::from_be_bytes([message[6], message[7]]));
    let mut pos = 12;
    for _ in 0..questions {
        pos = skip_name(message, pos)? + 4;
    }
    let mut found = Vec::new();
    let mut ttl = u32::MAX;
    for _ in 0..answers {
        pos = skip_name(message, pos)?;
        let fixed = message
            .get(pos..pos + 10)
            .ok_or_else(|| invalid("resposta DNS cortada"))?;
        let kind = u16::from_be_bytes([fixed[0], fixed[1]]);
        let record_ttl = u32::from_be_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
        let length = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
        pos += 10;
        let data = message
            .get(pos..pos + length)
            .ok_or_else(|| invalid("resposta DNS cortada"))?;
        pos += length;
        if kind == 1 && length == 4 {
            found.push(IpAddr::V4(Ipv4Addr::new(
                data[0], data[1], data[2], data[3],
            )));
            ttl = ttl.min(record_ttl);
        }
    }
    if found.is_empty() {
        Ok(Reply::Empty)
    } else {
        Ok(Reply::Found(found, Duration::from_secs(u64::from(ttl))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uma resposta gravada de um servidor: `alvo.rede.local` é um CNAME de `web.rede.local`,
    /// que tem dois endereços (o nome do CNAME e o dos A usam ponteiros de compressão).
    fn gravada(id: u16) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&id.to_be_bytes());
        m.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 3, 0, 0, 0, 0]);
        for label in ["alvo", "rede", "local"] {
            m.push(label.len() as u8);
            m.extend_from_slice(label.as_bytes());
        }
        m.push(0);
        m.extend_from_slice(&[0, 1, 0, 1]);
        // CNAME para web.rede.local (o alvo tem um ponteiro para "rede.local").
        m.extend_from_slice(&[0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 6, 3, b'w', b'e', b'b']);
        m.extend_from_slice(&[0xC0, 17]);
        // Dois A de web.rede.local (o ponteiro 45 aponta para o "web" dentro do CNAME).
        for ip in [[10, 99, 0, 10], [10, 99, 0, 11]] {
            m.extend_from_slice(&[0xC0, 45, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4]);
            m.extend_from_slice(&ip);
        }
        m
    }

    #[test]
    fn a_consulta_tem_o_formato_do_dns_sobre_tcp() {
        let q = build_query(0x1234, "alvo.rede.local").unwrap();
        assert_eq!(u16::from_be_bytes([q[0], q[1]]) as usize, q.len() - 2);
        assert_eq!(&q[2..4], &[0x12, 0x34]);
        assert_eq!(&q[4..6], &[0x01, 0x00]);
        assert_eq!(&q[6..8], &[0, 1]);
        assert_eq!(&q[14..19], b"\x04alvo");
        assert_eq!(&q[q.len() - 4..], &[0, 1, 0, 1]);
        assert!(build_query(1, "a..b").is_err());
        assert!(build_query(1, &"a".repeat(64)).is_err());
    }

    #[test]
    fn a_resposta_gravada_com_cname_e_compressao_entrega_os_dois_enderecos() {
        let Reply::Found(list, ttl) = parse_reply(7, &gravada(7)).unwrap() else {
            panic!("devia ter achado");
        };
        assert_eq!(
            list,
            vec![
                IpAddr::V4(Ipv4Addr::new(10, 99, 0, 10)),
                IpAddr::V4(Ipv4Addr::new(10, 99, 0, 11))
            ]
        );
        assert_eq!(ttl, Duration::from_secs(30));
    }

    #[test]
    fn respostas_erradas_sao_recusadas() {
        assert!(parse_reply(8, &gravada(7)).is_err(), "id diferente");
        let mut nx = gravada(7);
        nx[3] = 0x83; // NXDOMAIN
        assert!(matches!(parse_reply(7, &nx), Ok(Reply::Empty)));
        let mut falha = gravada(7);
        falha[3] = 0x82; // SERVFAIL
        assert!(parse_reply(7, &falha).is_err());
        let cortada = gravada(7);
        assert!(parse_reply(7, &cortada[..cortada.len() - 3]).is_err());
        assert!(parse_reply(7, &cortada[..5]).is_err());
    }

    #[test]
    fn o_escopo_decide_que_nomes_vao_para_a_vpn() {
        let ip = Ipv4Addr::new(10, 99, 0, 1);
        let todos = DnsOverTcp::new(vec![ip], vec![], DnsScope::All);
        assert!(todos.handles("exemplo.com"));
        let dominios = DnsOverTcp::new(
            vec![ip],
            vec!["Rede.Local.".into()],
            DnsScope::Domains(vec!["corp.interna".into()]),
        );
        assert!(dominios.handles("servidor"), "sem ponto vai para a VPN");
        assert!(dominios.handles("a.corp.interna"));
        assert!(dominios.handles("x.rede.local"));
        assert!(dominios.handles("corp.interna"));
        assert!(!dominios.handles("exemplo.com"));
        assert!(!dominios.handles("xcorp.interna"));
        assert_eq!(
            dominios.candidates("servidor"),
            vec!["servidor.rede.local".to_owned(), "servidor".to_owned()]
        );
        assert_eq!(dominios.candidates("a.b"), vec!["a.b".to_owned()]);
    }
}
