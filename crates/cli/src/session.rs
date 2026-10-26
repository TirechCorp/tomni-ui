//! O app em modo de ferramentas, como processo filho.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

use crate::{Error, PREFIX, PROTOCOL, TOOLS_ENV};

/// Linhas da saída de erro do app guardadas para as mensagens de erro.
const STDERR_TAIL: usize = 30;

/// Espera padrão por cada resposta do app; `TOMNI_UI_TIMEOUT` (segundos) muda.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Primeira resposta do app: as telas e a janela configurada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// Telas registradas, a padrão primeiro.
    pub views: Vec<String>,
    /// Tamanho da janela em `App::size`.
    pub size: (u32, u32),
    /// Tema fixado com `App::theme`, se houver.
    pub theme: Option<String>,
}

/// A tela aberta agora.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    /// Nome da tela.
    pub view: String,
    /// Tamanho.
    pub size: (u32, u32),
    /// `light` ou `dark`.
    pub theme: String,
}

/// App rodando em modo de ferramentas. Fecha a entrada padrão do app ao sair, e o mata se ele
/// não terminar.
#[derive(Debug)]
pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    replies: Receiver<Result<Value, String>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    exe: PathBuf,
    modified: Option<SystemTime>,
    timeout: Duration,
    /// O que o app informou ao começar.
    pub hello: Hello,
    /// A tela aberta, depois do primeiro [`Session::open`].
    pub screen: Option<Screen>,
}

impl Session {
    /// Roda `exe` em modo de ferramentas e espera a primeira resposta. O que o app imprime fora
    /// do protocolo vai para a saída de erro.
    pub fn start(exe: &Path) -> Result<Self, Error> {
        let mut child = Command::new(exe)
            .env(TOOLS_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Other(format!("não deu para rodar {}: {e}", exe.display())))?;
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let pipe = child.stderr.take().expect("stderr em pipe");
        let tail = Arc::clone(&stderr);
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                eprintln!("{line}");
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
        let (sender, replies) = mpsc::channel();
        let pipe = child.stdout.take().expect("stdout em pipe");
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                match line.strip_prefix(PREFIX) {
                    Some(json) => {
                        let reply = serde_json::from_str(json).map_err(|e| e.to_string());
                        if sender.send(reply).is_err() {
                            break;
                        }
                    }
                    // `println!` do app: não pode sujar a saída da CLI nem o JSON-RPC do MCP.
                    None => eprintln!("{line}"),
                }
            }
        });
        let timeout = std::env::var("TOMNI_UI_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .map_or(TIMEOUT, Duration::from_secs);
        let mut session = Self {
            stdin: child.stdin.take(),
            child,
            replies,
            stderr,
            exe: exe.to_owned(),
            modified: modified(exe),
            timeout,
            hello: Hello {
                views: Vec::new(),
                size: (0, 0),
                theme: None,
            },
            screen: None,
        };
        let hello = session.receive().map_err(|e| {
            Error::Other(format!(
                "{} não atendeu o modo de ferramentas: o `main` precisa terminar em \
                 `tomni_ui::App::new(tela).run()`, com a feature `tools` do tomni-ui ligada (é \
                 padrão). {e}",
                exe.display()
            ))
        })?;
        if hello["protocol"].as_u64() != Some(PROTOCOL) {
            return Err(Error::Other(format!(
                "o app fala o protocolo {} e esta ferramenta, o {PROTOCOL}: use a mesma versão do \
                 tomni-ui e do tomni-ui-cli",
                hello["protocol"]
            )));
        }
        session.hello = Hello {
            views: hello["views"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            size: (
                u32_of(&hello["width"]).unwrap_or(800),
                u32_of(&hello["height"]).unwrap_or(600),
            ),
            theme: hello["theme"].as_str().map(str::to_owned),
        };
        Ok(session)
    }

    /// Executável que está rodando.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Se `exe` é outro executável, ou o mesmo recompilado depois que a sessão começou.
    pub fn is_stale(&self, exe: &Path) -> bool {
        self.exe != exe || self.modified != modified(exe)
    }

    /// Manda um pedido e espera a resposta.
    pub fn request(&mut self, request: &Value) -> Result<Value, Error> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Other("a sessão já foi encerrada".into()))?;
        let sent = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
        if let Err(e) = sent {
            return Err(Error::Other(format!(
                "o app fechou a entrada ({e}){}",
                self.exit_details()
            )));
        }
        let reply = self.receive()?;
        if reply["ok"] == true {
            Ok(reply)
        } else {
            Err(Error::App(
                reply["error"]
                    .as_str()
                    .unwrap_or("o app recusou o pedido")
                    .to_owned(),
            ))
        }
    }

    fn receive(&mut self) -> Result<Value, Error> {
        match self.replies.recv_timeout(self.timeout) {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(e)) => Err(Error::Other(format!("resposta inválida do app: {e}"))),
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.child.kill();
                Err(Error::Other(format!(
                    "o app não respondeu em {} s e foi encerrado (aumente com \
                     TOMNI_UI_TIMEOUT=segundos){}",
                    self.timeout.as_secs(),
                    self.stderr_tail()
                )))
            }
            Err(RecvTimeoutError::Disconnected) => Err(Error::Other(format!(
                "o app terminou{}",
                self.exit_details()
            ))),
        }
    }

    /// Abre `view` (ou a padrão) do zero, com o tamanho e o tema pedidos ou os do app.
    pub fn open(
        &mut self,
        view: Option<&str>,
        size: Option<(u32, u32)>,
        theme: Option<&str>,
    ) -> Result<&Screen, Error> {
        let mut request = json!({ "cmd": "open" });
        if let Some(view) = view {
            request["view"] = json!(view);
        }
        if let Some((width, height)) = size {
            request["width"] = json!(width);
            request["height"] = json!(height);
        }
        if let Some(theme) = theme {
            request["theme"] = json!(theme);
        }
        let reply = self.request(&request)?;
        let screen = Screen {
            view: reply["view"].as_str().unwrap_or_default().to_owned(),
            size: (
                u32_of(&reply["size"][0]).unwrap_or_default(),
                u32_of(&reply["size"][1]).unwrap_or_default(),
            ),
            theme: reply["theme"].as_str().unwrap_or("light").to_owned(),
        };
        Ok(self.screen.insert(screen))
    }

    /// Muda o tamanho da tela aberta, sem perder o estado.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        self.request(&json!({ "cmd": "resize", "width": width, "height": height }))?;
        if let Some(screen) = &mut self.screen {
            screen.size = (width, height);
        }
        Ok(())
    }

    /// Troca o tema da tela aberta, sem perder o estado.
    pub fn set_theme(&mut self, theme: &str) -> Result<(), Error> {
        self.request(&json!({ "cmd": "theme", "theme": theme }))?;
        if let Some(screen) = &mut self.screen {
            theme.clone_into(&mut screen.theme);
        }
        Ok(())
    }

    /// Grava um PNG da tela aberta em `path`.
    pub fn screenshot(&mut self, path: &Path) -> Result<(), Error> {
        self.request(&json!({ "cmd": "screenshot", "path": path }))
            .map(drop)
    }

    /// Árvore de widgets (`tomni_ui::Node` em JSON).
    pub fn tree(&mut self) -> Result<Value, Error> {
        Ok(self.request(&json!({ "cmd": "tree" }))?["tree"].take())
    }

    /// Problemas do lint (`tomni_ui::lint::Issue` em JSON), menos os das regras em `allow`.
    pub fn lint(&mut self, allow: &[String]) -> Result<Vec<Value>, Error> {
        let mut reply = self.request(&json!({ "cmd": "lint" }))?;
        let issues = match reply["issues"].take() {
            Value::Array(issues) => issues,
            _ => Vec::new(),
        };
        Ok(issues
            .into_iter()
            .filter(|issue| {
                !allow
                    .iter()
                    .any(|rule| issue["rule"].as_str() == Some(rule.as_str()))
            })
            .collect())
    }

    /// Roda as ações (`click`, `type`, `key`...) e devolve `repaints` e `notes`.
    pub fn act(&mut self, actions: &Value) -> Result<Value, Error> {
        let mut reply = self.request(&json!({ "cmd": "act", "actions": actions }))?;
        Ok(json!({ "repaints": reply["repaints"].take(), "notes": reply["notes"].take() }))
    }

    fn stderr_tail(&self) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        if tail.is_empty() {
            String::new()
        } else {
            let lines: Vec<&str> = tail.iter().map(String::as_str).collect();
            format!("\nsaída de erro do app:\n{}", lines.join("\n"))
        }
    }

    fn exit_details(&mut self) -> String {
        // A thread da saída de erro pode estar terminando de ler.
        let deadline = Instant::now() + Duration::from_millis(500);
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break None,
            }
        };
        let status = status.map_or(String::new(), |s| format!(" ({s})"));
        format!("{status}{}", self.stderr_tail())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Sem entrada, o app sai do laço de pedidos e termina.
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn modified(exe: &Path) -> Option<SystemTime> {
    std::fs::metadata(exe).and_then(|m| m.modified()).ok()
}

fn u32_of(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|v| u32::try_from(v).ok())
}
