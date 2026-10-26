//! `Terminal`: emulador de terminal (cores, cursor, seleção, histórico),
//! ligado a uma conexão. `cargo run -p tomni-ui --example terminal --features terminal`.
//!
//! A conexão aqui é um shell de brincadeira escrito no próprio exemplo, um `Transport` que
//! ecoa as teclas e responde a `ajuda`, `cores`, `eco`, `titulo`, `sino`, `links`, `limpar` e
//! `sair`.
//! Num app de verdade, troque por `Process` (um shell local), `Telnet`, `Ssh` ou `Serial`.
//!
//! O Tab vai ao programa; F6 leva o foco ao próximo widget. Ctrl+Shift+C copia, Ctrl+Shift+V
//! cola (e Ctrl+C copia quando há seleção). A roda e Shift+PgUp/PgDn rolam o histórico. O
//! Ctrl+clique abre um link (`links` escreve um do texto e um marcado pelo programa); o clique
//! direito sobre ele oferece abrir e copiar.

use std::io;

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Inbox, Status, Transport};

/// O prompt, em verde e azul como o do bash.
const PROMPT: &str = "\x1b[1;32mdemo\x1b[0m:\x1b[1;34m~\x1b[0m$ ";

/// Shell de brincadeira: guarda a linha digitada e responde no Enter.
struct Demo {
    inbox: Inbox,
    line: String,
}

impl Demo {
    fn new() -> Self {
        let demo = Self {
            inbox: Inbox::new(),
            line: String::new(),
        };
        demo.say("Tomni UI \x1b[1mTerminal\x1b[0m: digite \x1b[33majuda\x1b[0m.\r\n");
        demo.say(PROMPT);
        demo
    }

    fn say(&self, text: &str) {
        self.inbox.push(text.as_bytes());
    }

    fn run(&mut self) {
        let line = std::mem::take(&mut self.line);
        let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        match command {
            "" => {}
            "ajuda" => self.say(
                "ajuda          esta lista\r\n\
                 cores          as 16 cores do esquema\r\n\
                 eco TEXTO      repete o texto\r\n\
                 titulo TEXTO   muda o título (OSC 0)\r\n\
                 sino           toca a campainha (pisca a tela)\r\n\
                 links          escreve dois links\r\n\
                 limpar         limpa a tela\r\n\
                 sair           fecha a conexão\r\n",
            ),
            "cores" => {
                let mut text = String::new();
                for base in [40, 100] {
                    for i in 0..8 {
                        text.push_str(&format!("\x1b[{}m    ", base + i));
                    }
                    text.push_str("\x1b[0m\r\n");
                }
                self.say(&text);
            }
            "eco" => self.say(&format!("{rest}\r\n")),
            "titulo" => self.say(&format!("\x1b]0;{rest}\x07")),
            "sino" => self.say("\x07"),
            "links" => self.say(
                "Site: https://example.com/tomni-ui\r\n\
                 Marcado pelo programa: \x1b]8;;https://example.com/ajuda\x1b\\a ajuda\x1b]8;;\x1b\\\r\n",
            ),
            "limpar" => self.say("\x1b[2J\x1b[H"),
            "sair" => {
                self.inbox.close(None);
                return;
            }
            outro => self.say(&format!("{outro}: comando não encontrado\r\n")),
        }
        self.say(PROMPT);
    }
}

impl Transport for Demo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        // Setas e outras sequências de escape não fazem nada aqui.
        if bytes.first() == Some(&0x1b) || self.inbox.is_closed() {
            return Ok(());
        }
        for c in String::from_utf8_lossy(bytes).chars() {
            match c {
                '\r' => {
                    self.say("\r\n");
                    self.run();
                }
                '\x7f' | '\x08' => {
                    if self.line.pop().is_some() {
                        self.say("\x08 \x08");
                    }
                }
                '\x03' => {
                    self.line.clear();
                    self.say("^C\r\n");
                    self.say(PROMPT);
                }
                c if !c.is_control() => {
                    self.line.push(c);
                    self.say(c.encode_utf8(&mut [0; 4]));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.inbox.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.inbox.pending()
    }
}

/// Um console que ocupa a janela, com o título que o programa escolhe por cima.
pub fn console() -> impl View {
    let titulo = signal(String::from("demo"));
    let aberto = signal(String::from("nenhum"));
    Form::new("Terminal").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(Text::new(move || format!("Título: {}", titulo.get())).id("titulo"))
            .child(Text::new(move || format!("Link aberto: {}", aberto.get())).id("link"))
            .child(
                Terminal::new(Demo::new())
                    .id("console")
                    .grow(1)
                    .on_title(move |t| titulo.set(t))
                    .on_link(move |link| aberto.set(link.uri.clone())),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(console).size(800, 600).run()
}
