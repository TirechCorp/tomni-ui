//! Uma porta serial, pelo `serial2`: o console de um roteador, de uma placa ou de um
//! modem.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use serial2::Settings;
pub use serial2::{CharSize, FlowControl, Parity, SerialPort, StopBits};

use crate::transport::{Inbox, Status, Transport, closed_error, error_message};

/// De quanto em quanto tempo as threads conferem se o `Serial` foi solto.
const POLL: Duration = Duration::from_millis(100);

/// Uma porta serial aberta: `/dev/ttyUSB0` no Linux, `/dev/tty.usbserial-…` no macOS, `COM3`
/// no Windows.
///
/// O padrão: 9600 bauds, 8 bits, sem paridade, 1 bit de parada e controle de
/// fluxo XON/XOFF. A porta fica em modo cru: o Enter vai como CR, e o eco é do aparelho.
///
/// ```no_run
/// use tomni_ui_terminal::{Serial, Terminal};
/// use tomni_ui_terminal::serial::FlowControl;
///
/// let roteador = Terminal::new(Serial::open("/dev/ttyUSB0", 115_200)).id("roteador");
/// let placa = Serial::builder("COM3")
///     .baud_rate(9600)
///     .flow_control(FlowControl::None)
///     .open();
/// ```
#[derive(Debug)]
pub struct Serial {
    inbox: Inbox,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    /// Como abrir de novo; `None` numa porta dada ao [`Serial::from_port`].
    builder: Option<SerialBuilder>,
}

#[derive(Debug)]
enum Command {
    Data(Vec<u8>),
    Close,
}

/// A porta e a configuração de um [`Serial`], antes de abrir.
#[derive(Clone, Debug)]
pub struct SerialBuilder {
    path: PathBuf,
    baud_rate: u32,
    char_size: CharSize,
    parity: Parity,
    stop_bits: StopBits,
    flow_control: FlowControl,
}

impl SerialBuilder {
    /// A porta `path`, com a configuração padrão (9600 8N1, XON/XOFF).
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            baud_rate: 9600,
            char_size: CharSize::Bits8,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::XonXoff,
        }
    }

    /// A velocidade, em bauds (9600, 115200).
    pub fn baud_rate(mut self, baud_rate: u32) -> Self {
        self.baud_rate = baud_rate;
        self
    }

    /// Os bits de dados (8 é o padrão).
    pub fn char_size(mut self, char_size: CharSize) -> Self {
        self.char_size = char_size;
        self
    }

    /// A paridade (nenhuma é o padrão).
    pub fn parity(mut self, parity: Parity) -> Self {
        self.parity = parity;
        self
    }

    /// Os bits de parada (1 é o padrão).
    pub fn stop_bits(mut self, stop_bits: StopBits) -> Self {
        self.stop_bits = stop_bits;
        self
    }

    /// O controle de fluxo (XON/XOFF é o padrão).
    pub fn flow_control(mut self, flow_control: FlowControl) -> Self {
        self.flow_control = flow_control;
        self
    }

    /// Abre a porta. Não falha: um erro ("não foi possível abrir /dev/ttyUSB0: permissão
    /// negada") aparece no terminal, como o da conexão do `Telnet`.
    pub fn open(self) -> Serial {
        let inbox = Inbox::new();
        let (commands, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut serial = Serial {
            inbox,
            commands,
            stop,
            builder: Some(self.clone()),
        };
        self.launch(&mut serial, receiver);
        serial
    }

    /// Abre a porta e fecha a fila com o erro, se não deu.
    fn launch(&self, serial: &mut Serial, commands: Receiver<Command>) {
        if let Err(error) = self.start(serial, commands) {
            let path = self.path.display();
            serial.inbox.close(Some(format!(
                "não foi possível abrir {path}: {}",
                error_message(&error)
            )));
        }
    }

    fn start(&self, serial: &Serial, commands: Receiver<Command>) -> io::Result<()> {
        let mut port = SerialPort::open(&self.path, |mut settings: Settings| {
            settings.set_raw();
            settings.set_baud_rate(self.baud_rate)?;
            settings.set_char_size(self.char_size);
            settings.set_parity(self.parity);
            settings.set_stop_bits(self.stop_bits);
            settings.set_flow_control(self.flow_control);
            Ok(settings)
        })?;
        Serial::run(&mut port, serial, commands)
    }
}

impl Serial {
    /// Abre `path` a `baud_rate` bauds, com o resto da configuração padrão (8N1, XON/XOFF).
    pub fn open(path: impl AsRef<Path>, baud_rate: u32) -> Self {
        SerialBuilder::new(path).baud_rate(baud_rate).open()
    }

    /// A porta `path`, para mudar a configuração antes de abrir.
    pub fn builder(path: impl AsRef<Path>) -> SerialBuilder {
        SerialBuilder::new(path)
    }

    /// Usa uma porta já aberta e configurada (um par de pseudoterminais, nos testes).
    pub fn from_port(mut port: SerialPort) -> Self {
        let (commands, receiver) = mpsc::channel();
        let serial = Self {
            inbox: Inbox::new(),
            commands,
            stop: Arc::new(AtomicBool::new(false)),
            builder: None,
        };
        if let Err(error) = Self::run(&mut port, &serial, receiver) {
            serial.inbox.close(Some(error_message(&error)));
        }
        serial
    }

    /// Solta as threads de leitura e de escrita.
    fn run(port: &mut SerialPort, serial: &Serial, commands: Receiver<Command>) -> io::Result<()> {
        port.set_read_timeout(POLL)?;
        port.set_write_timeout(POLL)?;
        let writer = port.try_clone()?;
        let reader = port.try_clone()?;
        let (read_inbox, read_stop) = (serial.inbox.clone(), Arc::clone(&serial.stop));
        thread::Builder::new()
            .name("tomni-ui-serial-leitura".into())
            .spawn(move || read_loop(&reader, &read_inbox, &read_stop))?;
        let (write_inbox, write_stop) = (serial.inbox.clone(), Arc::clone(&serial.stop));
        thread::Builder::new()
            .name("tomni-ui-serial-escrita".into())
            .spawn(move || write_loop(&writer, &commands, &write_inbox, &write_stop))?;
        Ok(())
    }
}

impl Drop for Serial {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.send(Command::Close);
    }
}

impl Transport for Serial {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.commands
            .send(Command::Data(bytes.to_vec()))
            .map_err(|_| closed_error())
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.inbox.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.inbox.pending()
    }

    fn reopen(&mut self) -> bool {
        let Some(builder) = self.builder.clone() else {
            return false;
        };
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.send(Command::Close);
        self.stop = Arc::new(AtomicBool::new(false));
        self.inbox.reopen();
        let (commands, receiver) = mpsc::channel();
        self.commands = commands;
        builder.launch(self, receiver);
        true
    }

    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        // Duas pessoas na mesma porta serial brigam pelos bytes: só a mesma porta reaberta
        // depois de encerrada (`reopen`) faz sentido, então duplicar não abre outra.
        None
    }
}

fn waiting(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

fn read_loop(port: &SerialPort, inbox: &Inbox, stop: &AtomicBool) {
    let mut buf = vec![0; 4096];
    while !stop.load(Ordering::Acquire) {
        match port.read(&mut buf) {
            Ok(0) => return inbox.close(None),
            Ok(n) => inbox.push(&buf[..n]),
            Err(error) if waiting(&error) => {}
            Err(error) => return inbox.close(Some(error_message(&error))),
        }
    }
}

fn write_loop(port: &SerialPort, commands: &Receiver<Command>, inbox: &Inbox, stop: &AtomicBool) {
    for command in commands {
        let Command::Data(bytes) = command else {
            return;
        };
        // Com o controle de fluxo, o aparelho pode segurar a escrita: espera sem desistir
        // até o `Serial` ser solto.
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            match port.write(rest) {
                Ok(n) => rest = &rest[n..],
                Err(error) if waiting(&error) => {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                }
                Err(error) => return inbox.close(Some(error_message(&error))),
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    fn ler(serial: &mut Serial, n: usize) -> Vec<u8> {
        let limite = Instant::now() + Duration::from_secs(5);
        let mut lido = Vec::new();
        while lido.len() < n && Instant::now() < limite {
            serial.read(&mut lido, usize::MAX);
            thread::sleep(Duration::from_millis(5));
        }
        lido
    }

    #[test]
    fn conversa_pelo_par_de_portas() {
        let (a, aparelho) = SerialPort::pair().unwrap();
        let mut serial = Serial::from_port(a);
        serial.write(b"show version\r").unwrap();
        let mut recebido = [0; 64];
        let limite = Instant::now() + Duration::from_secs(5);
        let mut n = 0;
        while n < 13 && Instant::now() < limite {
            n += aparelho.read(&mut recebido[n..]).unwrap_or(0);
        }
        assert_eq!(&recebido[..n], b"show version\r");
        aparelho.write_all("roteador olá\r\n".as_bytes()).unwrap();
        assert_eq!(ler(&mut serial, 15), "roteador olá\r\n".as_bytes());
    }

    #[test]
    fn porta_que_nao_existe() {
        let mut serial = Serial::open("/dev/tomni-ui-porta-que-nao-existe", 9600);
        let mut lido = Vec::new();
        let Status::Closed(Some(mensagem)) = serial.read(&mut lido, 16) else {
            panic!("devia fechar com erro");
        };
        assert!(
            mensagem.starts_with("não foi possível abrir /dev/tomni-ui-porta-que-nao-existe: "),
            "{mensagem}"
        );
    }
}
