//! Senha, `auth-token`, desafios (`SCRV1` e `CRV1`) e as mensagens de controle do servidor.
//!
//! O usuário e a senha vão no key-method 2 ([`crate::keymethod`]). O que muda é o que se põe no
//! campo da senha:
//!
//! - a senha em si;
//! - `SCRV1:<senha em base64>:<resposta em base64>` quando o `.ovpn` tem `static-challenge`;
//! - `CRV1::<state_id>::<resposta>` depois de um desafio dinâmico
//!   (`AUTH_FAILED,CRV1:<flags>:<state_id>:<usuário em base64>:<texto>`);
//! - o `auth-token` que o servidor empurrou, nas renegociações e reconexões.
//!
//! Este módulo também lê as mensagens de texto que o servidor manda pelo canal de controle
//! ([`parse_server_message`]). Nada aqui limita o número de tentativas: quem decide perguntar de
//! novo ou desistir é o motor.

use std::fmt;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use zeroize::Zeroizing;

use crate::config::AuthUserPass;

/// O usuário e a senha do `auth-user-pass` (vindos do cofre ou da pergunta à pessoa).
#[derive(Clone)]
pub struct Credentials {
    /// O usuário.
    pub user: String,
    /// A senha.
    pub password: Zeroizing<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Credentials {{ user: {:?}, password: (oculta) }}",
            self.user
        )
    }
}

impl Credentials {
    /// Junta o usuário e a senha.
    pub fn new(user: impl Into<String>, password: impl Into<String>) -> Credentials {
        Credentials {
            user: user.into(),
            password: Zeroizing::new(password.into()),
        }
    }

    /// As credenciais que o `.ovpn` já traz (`auth-user-pass arquivo`), se completas.
    pub fn from_config(auth: &AuthUserPass) -> Option<Credentials> {
        match auth {
            AuthUserPass::Credentials {
                user,
                password: Some(p),
            } => Some(Credentials {
                user: user.clone(),
                password: p.clone(),
            }),
            _ => None,
        }
    }
}

/// O texto do campo da senha para um `static-challenge`: `SCRV1:<senha>:<resposta>`, as duas em
/// base64.
pub fn static_challenge_password(password: &str, answer: &str) -> Zeroizing<String> {
    Zeroizing::new(format!(
        "SCRV1:{}:{}",
        B64.encode(password.as_bytes()),
        B64.encode(answer.as_bytes())
    ))
}

/// O texto do campo da senha para a resposta de um desafio dinâmico: `CRV1::<state_id>::<resposta>`.
pub fn dynamic_challenge_password(state_id: &str, answer: &str) -> Zeroizing<String> {
    Zeroizing::new(format!("CRV1::{state_id}::{answer}"))
}

/// Lê um `SCRV1:<b64>:<b64>`: devolve a senha e a resposta. (O servidor de teste usa.)
pub fn parse_static_challenge_password(
    text: &str,
) -> Option<(Zeroizing<String>, Zeroizing<String>)> {
    let resto = text.strip_prefix("SCRV1:")?;
    let (senha, resposta) = resto.split_once(':')?;
    let decode = |s: &str| {
        let bytes = Zeroizing::new(B64.decode(s).ok()?);
        String::from_utf8(bytes.to_vec()).ok().map(Zeroizing::new)
    };
    Some((decode(senha)?, decode(resposta)?))
}

/// Lê um `CRV1::<state_id>::<resposta>`: devolve o `state_id` e a resposta.
pub fn parse_dynamic_challenge_password(text: &str) -> Option<(String, Zeroizing<String>)> {
    let resto = text.strip_prefix("CRV1::")?;
    let (state, resposta) = resto.split_once("::")?;
    Some((state.to_owned(), Zeroizing::new(resposta.to_owned())))
}

/// Um desafio dinâmico: `AUTH_FAILED,CRV1:<flags>:<state_id>:<usuário em base64>:<texto>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicChallenge {
    /// A flag `E`: a resposta aparece enquanto se digita.
    pub echo: bool,
    /// A flag `R`: o servidor espera uma resposta (sem ela, é só um aviso).
    pub response_required: bool,
    /// O identificador que o servidor guardou e quer de volta na resposta.
    pub state_id: String,
    /// O usuário (o da tentativa que gerou o desafio).
    pub user: String,
    /// O texto que a pessoa vê.
    pub text: String,
}

impl DynamicChallenge {
    /// A senha que responde ao desafio.
    pub fn answer(&self, answer: &str) -> Credentials {
        Credentials {
            user: self.user.clone(),
            password: dynamic_challenge_password(&self.state_id, answer),
        }
    }
}

/// O que o servidor disse ao recusar a autenticação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFailure {
    /// `AUTH_FAILED[,motivo]`: usuário ou senha recusados.
    Rejected {
        /// O motivo, se o servidor deu.
        reason: Option<String>,
    },
    /// `AUTH_FAILED,TEMP[backoff n,advance x]:motivo`: recusa passageira (tente de novo).
    Temporary {
        /// O motivo, se o servidor deu.
        reason: Option<String>,
        /// Quanto esperar antes de tentar de novo (`backoff`).
        backoff: Option<Duration>,
        /// O `advance` pedido: `no`, `addr` ou `remote`.
        advance: Option<String>,
    },
    /// `AUTH_FAILED,SESSION:motivo`: o `auth-token` não vale mais; use a senha de verdade.
    Session {
        /// O motivo, se o servidor deu.
        reason: Option<String>,
    },
    /// `AUTH_FAILED,CRV1:...`: o servidor quer uma resposta a um desafio.
    Challenge(DynamicChallenge),
}

impl fmt::Display for AuthFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let com_motivo = |f: &mut fmt::Formatter<'_>, base: &str, r: &Option<String>| match r {
            Some(m) if !m.is_empty() => write!(f, "{base}: {m}"),
            _ => f.write_str(base),
        };
        match self {
            Self::Rejected { reason } => {
                com_motivo(f, "usuário ou senha recusados pelo servidor", reason)
            }
            Self::Temporary { reason, .. } => {
                com_motivo(f, "o servidor recusou a autenticação por enquanto", reason)
            }
            Self::Session { reason } => {
                com_motivo(f, "o servidor não aceita mais o token da sessão", reason)
            }
            Self::Challenge(c) => write!(f, "o servidor fez uma pergunta: {}", c.text),
        }
    }
}

/// As mensagens de texto que o servidor manda pelo canal de controle depois do key-method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    /// `PUSH_REPLY,...` (o texto inteiro, para o [`crate::push::PushReader`]).
    PushReply(String),
    /// `AUTH_FAILED...`.
    AuthFailed(AuthFailure),
    /// `AUTH_PENDING[,timeout n]`: o servidor espera uma confirmação de fora.
    AuthPending {
        /// Quanto tempo o servidor espera, se disse.
        timeout: Option<Duration>,
    },
    /// `RESTART[,texto]`: reconectar (o servidor pode empurrar outro endereço).
    Restart(String),
    /// `HALT[,texto]`: parar de vez.
    Halt(String),
    /// `INFO_PRE,OPEN_URL:url` ou `WEB_AUTH::url`: abrir a página de login.
    OpenUrl(String),
    /// `INFO[_PRE],texto`: informação que o app só registra.
    Info(String),
    /// Qualquer outra mensagem (`PUSH_UPDATE`, `CR_TEXT`...).
    Other(String),
}

/// Lê uma mensagem de texto do servidor (sem o NUL final).
pub fn parse_server_message(text: &str) -> ServerMessage {
    let text = text.trim_end_matches('\0');
    let (nome, resto) = match text.split_once(',') {
        Some((n, r)) => (n, Some(r)),
        None => (text, None),
    };
    match nome {
        "PUSH_REPLY" => ServerMessage::PushReply(text.to_owned()),
        "AUTH_FAILED" => ServerMessage::AuthFailed(parse_auth_failed(resto)),
        "AUTH_PENDING" => {
            let timeout = resto
                .and_then(|r| r.trim().strip_prefix("timeout"))
                .and_then(|n| n.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            ServerMessage::AuthPending { timeout }
        }
        "RESTART" => ServerMessage::Restart(resto.unwrap_or("").to_owned()),
        "HALT" => ServerMessage::Halt(resto.unwrap_or("").to_owned()),
        "INFO" | "INFO_PRE" => {
            let corpo = resto.unwrap_or("");
            if let Some(url) = corpo.strip_prefix("OPEN_URL:") {
                ServerMessage::OpenUrl(url.to_owned())
            } else if let Some(r) = corpo.strip_prefix("WEB_AUTH:") {
                // `WEB_AUTH:<flags>:<url>`
                ServerMessage::OpenUrl(r.split_once(':').map_or(r, |(_, u)| u).to_owned())
            } else {
                ServerMessage::Info(corpo.to_owned())
            }
        }
        _ => ServerMessage::Other(text.to_owned()),
    }
}

fn parse_auth_failed(resto: Option<&str>) -> AuthFailure {
    let Some(resto) = resto else {
        return AuthFailure::Rejected { reason: None };
    };
    if let Some(c) = resto.strip_prefix("CRV1:")
        && let Some(desafio) = parse_crv1(c)
    {
        return AuthFailure::Challenge(desafio);
    }
    if let Some(r) = resto.strip_prefix("SESSION") {
        return AuthFailure::Session {
            reason: motivo(r.strip_prefix(':').unwrap_or(r)),
        };
    }
    if let Some(r) = resto.strip_prefix("TEMP") {
        let (flags, razao) = match r.strip_prefix('[') {
            Some(dentro) => match dentro.split_once(']') {
                Some((f, depois)) => (f, depois),
                None => ("", r),
            },
            None => ("", r),
        };
        let mut backoff = None;
        let mut advance = None;
        for flag in flags.split(',') {
            let flag = flag.trim();
            if let Some(n) = flag.strip_prefix("backoff") {
                backoff = n.trim().parse::<u64>().ok().map(Duration::from_secs);
            } else if let Some(a) = flag.strip_prefix("advance") {
                advance = Some(a.trim().to_owned()).filter(|a| !a.is_empty());
            }
        }
        return AuthFailure::Temporary {
            reason: motivo(razao.strip_prefix(':').unwrap_or(razao)),
            backoff,
            advance,
        };
    }
    AuthFailure::Rejected {
        reason: motivo(resto),
    }
}

fn motivo(texto: &str) -> Option<String> {
    let t = texto.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// `<flags>:<state_id>:<usuário em base64>:<texto>`; o texto pode ter `:`.
fn parse_crv1(texto: &str) -> Option<DynamicChallenge> {
    let mut partes = texto.splitn(4, ':');
    let flags = partes.next()?;
    let state_id = partes.next()?.to_owned();
    let usuario = partes.next()?;
    let texto = partes.next()?.to_owned();
    let user = String::from_utf8(B64.decode(usuario).ok()?).ok()?;
    Some(DynamicChallenge {
        echo: flags.split(',').any(|f| f == "E"),
        response_required: flags.split(',').any(|f| f == "R"),
        state_id,
        user,
        text: texto,
    })
}

/// O `auth-token` empurrado pelo servidor.
#[derive(Clone)]
struct Token {
    value: Zeroizing<String>,
    user: Option<String>,
}

/// O que vai nos campos de usuário e senha do key-method, e como isso muda durante a sessão.
#[derive(Clone, Default)]
pub struct AuthState {
    credentials: Option<Credentials>,
    token: Option<Token>,
}

impl fmt::Debug for AuthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthState")
            .field("credentials", &self.credentials)
            .field("token", &self.token.as_ref().map(|_| "(oculto)"))
            .finish()
    }
}

impl AuthState {
    /// Sem senha (o `.ovpn` não tem `auth-user-pass`).
    pub fn none() -> AuthState {
        AuthState::default()
    }

    /// Com o usuário e a senha dados.
    pub fn new(credentials: Credentials) -> AuthState {
        AuthState {
            credentials: Some(credentials),
            token: None,
        }
    }

    /// Com a senha de um `static-challenge`: o campo da senha vira `SCRV1:...`.
    pub fn with_static_challenge(credentials: &Credentials, answer: &str) -> AuthState {
        AuthState::new(Credentials {
            user: credentials.user.clone(),
            password: static_challenge_password(&credentials.password, answer),
        })
    }

    /// Guarda o `auth-token` (e o usuário que vai com ele, se o servidor disse).
    pub fn set_token(&mut self, token: Zeroizing<String>, user: Option<String>) {
        self.token = Some(Token { value: token, user });
    }

    /// Esquece o `auth-token` (o servidor o recusou); volta a valer a senha.
    pub fn drop_token(&mut self) {
        self.token = None;
    }

    /// Se há um `auth-token` guardado.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// O usuário e o `auth-token`, para guardar entre reconexões.
    pub fn token(&self) -> Option<(Option<&str>, &str)> {
        self.token
            .as_ref()
            .map(|t| (t.user.as_deref(), t.value.as_str()))
    }

    /// Se há como se autenticar (credenciais ou token).
    pub fn is_empty(&self) -> bool {
        self.credentials.is_none() && self.token.is_none()
    }

    /// Os campos do key-method: o token vale mais que a senha.
    pub fn wire(&self) -> (Option<Zeroizing<String>>, Option<Zeroizing<String>>) {
        if let Some(t) = &self.token {
            let user = t
                .user
                .clone()
                .or_else(|| self.credentials.as_ref().map(|c| c.user.clone()));
            return (user.map(Zeroizing::new), Some(t.value.clone()));
        }
        match &self.credentials {
            Some(c) => (
                Some(Zeroizing::new(c.user.clone())),
                Some(c.password.clone()),
            ),
            None => (None, None),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn scrv1_ida_e_volta() {
        let s = static_challenge_password("senha:com:dois-pontos", "123456");
        assert!(s.starts_with("SCRV1:"));
        assert_eq!(s.matches(':').count(), 2, "{}", s.as_str());
        let (senha, resposta) = parse_static_challenge_password(&s).unwrap();
        assert_eq!(senha.as_str(), "senha:com:dois-pontos");
        assert_eq!(resposta.as_str(), "123456");
        assert!(parse_static_challenge_password("SCRV1:@@:@@").is_none());
        assert!(parse_static_challenge_password("outra").is_none());
    }

    #[test]
    fn scrv1_valor_conhecido() {
        // "senha" = c2VuaGE=, "123" = MTIz
        assert_eq!(
            static_challenge_password("senha", "123").as_str(),
            "SCRV1:c2VuaGE=:MTIz"
        );
    }

    #[test]
    fn crv1_ida_e_volta() {
        let s = dynamic_challenge_password("abc123", "resposta: com ::");
        assert_eq!(s.as_str(), "CRV1::abc123::resposta: com ::");
        let (id, r) = parse_dynamic_challenge_password(&s).unwrap();
        assert_eq!(id, "abc123");
        assert_eq!(r.as_str(), "resposta: com ::");
        assert!(parse_dynamic_challenge_password("CRV1:x").is_none());
    }

    #[test]
    fn auth_failed_simples_e_com_motivo() {
        assert_eq!(
            parse_server_message("AUTH_FAILED"),
            ServerMessage::AuthFailed(AuthFailure::Rejected { reason: None })
        );
        assert_eq!(
            parse_server_message("AUTH_FAILED,senha vencida\0"),
            ServerMessage::AuthFailed(AuthFailure::Rejected {
                reason: Some("senha vencida".into())
            })
        );
        // O motivo pode ter vírgulas.
        assert_eq!(
            parse_server_message("AUTH_FAILED,a, b, c"),
            ServerMessage::AuthFailed(AuthFailure::Rejected {
                reason: Some("a, b, c".into())
            })
        );
    }

    #[test]
    fn auth_failed_temp_e_session() {
        let m = parse_server_message("AUTH_FAILED,TEMP[backoff 5,advance addr]:aguarde");
        assert_eq!(
            m,
            ServerMessage::AuthFailed(AuthFailure::Temporary {
                reason: Some("aguarde".into()),
                backoff: Some(Duration::from_secs(5)),
                advance: Some("addr".into()),
            })
        );
        let m = parse_server_message("AUTH_FAILED,TEMP");
        assert_eq!(
            m,
            ServerMessage::AuthFailed(AuthFailure::Temporary {
                reason: None,
                backoff: None,
                advance: None
            })
        );
        let m = parse_server_message("AUTH_FAILED,SESSION: token vencido");
        assert_eq!(
            m,
            ServerMessage::AuthFailed(AuthFailure::Session {
                reason: Some("token vencido".into())
            })
        );
    }

    #[test]
    fn auth_failed_crv1() {
        let usuario = B64.encode("maria");
        let m = parse_server_message(&format!(
            "AUTH_FAILED,CRV1:R,E:estado-9:{usuario}:Código: do app"
        ));
        let ServerMessage::AuthFailed(AuthFailure::Challenge(c)) = m else {
            panic!("não é desafio");
        };
        assert!(c.echo && c.response_required);
        assert_eq!(c.state_id, "estado-9");
        assert_eq!(c.user, "maria");
        assert_eq!(c.text, "Código: do app");
        let cred = c.answer("424242");
        assert_eq!(cred.user, "maria");
        assert_eq!(cred.password.as_str(), "CRV1::estado-9::424242");
        // Sem flags `E`.
        let m = parse_server_message(&format!("AUTH_FAILED,CRV1:R:x:{usuario}:t"));
        let ServerMessage::AuthFailed(AuthFailure::Challenge(c)) = m else {
            panic!()
        };
        assert!(!c.echo);
        // Malformada: vira recusa comum.
        assert!(matches!(
            parse_server_message("AUTH_FAILED,CRV1:R"),
            ServerMessage::AuthFailed(AuthFailure::Rejected { .. })
        ));
    }

    #[test]
    fn auth_pending_e_url() {
        assert_eq!(
            parse_server_message("AUTH_PENDING"),
            ServerMessage::AuthPending { timeout: None }
        );
        assert_eq!(
            parse_server_message("AUTH_PENDING,timeout 120"),
            ServerMessage::AuthPending {
                timeout: Some(Duration::from_secs(120))
            }
        );
        assert_eq!(
            parse_server_message("INFO_PRE,OPEN_URL:https://sso.exemplo/login?x=1"),
            ServerMessage::OpenUrl("https://sso.exemplo/login?x=1".into())
        );
        assert_eq!(
            parse_server_message("INFO,WEB_AUTH:x:https://sso.exemplo/a"),
            ServerMessage::OpenUrl("https://sso.exemplo/a".into())
        );
        assert_eq!(
            parse_server_message("INFO,CR_TEXT:E:digite"),
            ServerMessage::Info("CR_TEXT:E:digite".into())
        );
    }

    #[test]
    fn restart_halt_e_outras() {
        assert_eq!(
            parse_server_message("RESTART,mudou o servidor"),
            ServerMessage::Restart("mudou o servidor".into())
        );
        assert_eq!(
            parse_server_message("HALT"),
            ServerMessage::Halt(String::new())
        );
        assert_eq!(
            parse_server_message("PUSH_REPLY,ping 10"),
            ServerMessage::PushReply("PUSH_REPLY,ping 10".into())
        );
        assert!(matches!(
            parse_server_message("PUSH_UPDATE,ping 10"),
            ServerMessage::Other(_)
        ));
    }

    #[test]
    fn o_token_vale_mais_que_a_senha_e_pode_ser_descartado() {
        let mut a = AuthState::new(Credentials::new("maria", "segredo"));
        let (u, p) = a.wire();
        assert_eq!(
            (u.unwrap().as_str(), p.unwrap().as_str()),
            ("maria", "segredo")
        );
        a.set_token(Zeroizing::new("TOKEN".into()), None);
        let (u, p) = a.wire();
        assert_eq!(
            (u.unwrap().as_str(), p.unwrap().as_str()),
            ("maria", "TOKEN")
        );
        a.set_token(Zeroizing::new("T2".into()), Some("outro".into()));
        let (u, p) = a.wire();
        assert_eq!((u.unwrap().as_str(), p.unwrap().as_str()), ("outro", "T2"));
        assert!(a.has_token());
        assert!(!format!("{a:?}").contains("T2"));
        a.drop_token();
        let (_, p) = a.wire();
        assert_eq!(p.unwrap().as_str(), "segredo");
        assert!(AuthState::none().wire().0.is_none());
        assert!(AuthState::none().is_empty());
    }

    #[test]
    fn senha_com_static_challenge() {
        let cred = Credentials::new("maria", "segredo");
        let a = AuthState::with_static_challenge(&cred, "654321");
        let (u, p) = a.wire();
        assert_eq!(u.unwrap().as_str(), "maria");
        let (senha, resposta) = parse_static_challenge_password(&p.unwrap()).unwrap();
        assert_eq!((senha.as_str(), resposta.as_str()), ("segredo", "654321"));
    }

    #[test]
    fn credenciais_nao_vazam_no_debug() {
        let c = Credentials::new("maria", "segredo");
        assert!(!format!("{c:?}").contains("segredo"));
    }
}
