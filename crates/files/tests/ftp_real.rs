//! A matriz real do M31: o `Ftp` contra o vsftpd, o proftpd e o pure-ftpd de verdade, em
//! Docker (`testes/ftp/rodar.sh`). Fora do `cargo test --workspace`: todos os testes são
//! `#[ignore]`.
//!
//! A matriz é **servidor × {TLS explícito, TLS implícito, sem TLS} × {passivo, ativo} ×
//! {TLS automático, só 1.2, só 1.3}**. Cada caso faz login, lista, envia 1 MiB, baixa e
//! confere, retoma o envio e o download, renomeia, muda a data, muda as permissões e apaga. A
//! tabela sai na saída do teste (`--nocapture`). Os casos de TLS "automático" e "só 1.2" têm de
//! passar em todos os servidores; o "só 1.3" é só informação (o vsftpd e o proftpd com a
//! retomada exigida só funcionam no 1.3 se o OpenSSL deles retomar).
#![cfg(all(unix, feature = "ftp"))]

mod comum;

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui_files::ftp::{Ftp, FtpBuilder, FtpMode, FtpSecurity, TlsVersion};
use tomni_ui_files::{EntryKind, Fs};

use comum::dados;

const HOST: &str = "127.0.0.1";

struct Servidor {
    nome: &'static str,
    /// (porta, segurança, aceita FTP sem TLS na mesma porta)
    portas: &'static [(u16, FtpSecurity, bool)],
}

const SERVIDORES: &[Servidor] = &[
    Servidor {
        nome: "vsftpd",
        portas: &[
            (2121, FtpSecurity::Explicit, true),
            (2990, FtpSecurity::Implicit, false),
        ],
    },
    Servidor {
        nome: "proftpd",
        portas: &[
            (2122, FtpSecurity::Explicit, true),
            (2991, FtpSecurity::Implicit, false),
        ],
    },
    Servidor {
        nome: "pure-ftpd",
        portas: &[
            (2123, FtpSecurity::Explicit, true),
            (2124, FtpSecurity::Explicit, false),
        ],
    },
];

#[derive(Clone, Copy)]
struct Caso {
    servidor: &'static str,
    porta: u16,
    seguranca: FtpSecurity,
    modo: FtpMode,
    tls: TlsVersion,
}

impl Caso {
    fn nome(&self) -> String {
        let seguranca = match self.seguranca {
            FtpSecurity::None => "sem TLS",
            FtpSecurity::Implicit => "implícito",
            _ => "explícito",
        };
        let modo = match self.modo {
            FtpMode::Passive => "passivo",
            FtpMode::Active => "ativo",
        };
        let tls = match (self.seguranca, self.tls) {
            (FtpSecurity::None, _) => "-",
            (_, TlsVersion::Auto) => "auto",
            (_, TlsVersion::Only12) => "1.2",
            (_, TlsVersion::Only13) => "1.3",
        };
        format!(
            "{:<9} :{} {seguranca:<9} {modo:<7} TLS {tls}",
            self.servidor, self.porta
        )
    }

    /// Os casos que têm de passar (o "só 1.3" é só informação).
    fn obrigatorio(&self) -> bool {
        self.tls != TlsVersion::Only13
    }
}

fn casos() -> Vec<Caso> {
    let mut casos = Vec::new();
    for servidor in SERVIDORES {
        for &(porta, seguranca, aceita_claro) in servidor.portas {
            for modo in [FtpMode::Passive, FtpMode::Active] {
                for tls in [TlsVersion::Auto, TlsVersion::Only12, TlsVersion::Only13] {
                    casos.push(Caso {
                        servidor: servidor.nome,
                        porta,
                        seguranca,
                        modo,
                        tls,
                    });
                }
                if aceita_claro {
                    casos.push(Caso {
                        servidor: servidor.nome,
                        porta,
                        seguranca: FtpSecurity::None,
                        modo,
                        tls: TlsVersion::Auto,
                    });
                }
            }
        }
    }
    casos
}

/// O resultado de um passo que pode não existir no servidor.
type Passo = Result<String, String>;

fn conectar(caso: &Caso, perguntas: &Arc<AtomicUsize>) -> Result<Ftp, String> {
    let contar = perguntas.clone();
    FtpBuilder::new(HOST, caso.porta)
        .login("teste", "teste")
        .security(caso.seguranca)
        .mode(caso.modo)
        .tls_version(caso.tls)
        .timeout(Duration::from_secs(20))
        .check_certificate(move |_| {
            contar.fetch_add(1, Ordering::SeqCst);
            true
        })
        .connect()
        .map_err(|erro| format!("conectar: {erro}"))
}

fn gravar(ftp: &mut Ftp, caminho: &str, offset: u64, conteudo: &[u8]) -> Result<(), String> {
    let mut arquivo = ftp
        .open_write(caminho, offset)
        .map_err(|erro| format!("abrir {caminho} para gravar: {erro}"))?;
    arquivo
        .write_all(conteudo)
        .map_err(|erro| format!("gravar {caminho}: {erro}"))?;
    arquivo
        .finish()
        .map_err(|erro| format!("fechar {caminho}: {erro}"))
}

fn ler(ftp: &mut Ftp, caminho: &str, offset: u64) -> Result<Vec<u8>, String> {
    let mut arquivo = ftp
        .open_read(caminho, offset)
        .map_err(|erro| format!("abrir {caminho} para ler: {erro}"))?;
    let mut saida = Vec::new();
    arquivo
        .read_to_end(&mut saida)
        .map_err(|erro| format!("ler {caminho}: {erro}"))?;
    arquivo
        .finish()
        .map_err(|erro| format!("fechar {caminho}: {erro}"))?;
    Ok(saida)
}

/// Roda um caso; devolve as observações (`datas: ok`, `chmod: n/d`…) ou o primeiro erro.
fn rodar(caso: &Caso, indice: usize) -> Result<Vec<String>, String> {
    let perguntas = Arc::new(AtomicUsize::new(0));
    let mut ftp = conectar(caso, &perguntas)?;
    let mut notas = Vec::new();
    if let Some(aviso) = ftp.take_notice() {
        notas.push(aviso);
    }

    let lista = ftp.list("/").map_err(|erro| format!("listar: {erro}"))?;
    if !lista.iter().any(|e| e.name == "exemplo.txt") {
        return Err(format!(
            "a listagem não tem exemplo.txt: {:?}",
            lista.iter().map(|e| &e.name).collect::<Vec<_>>()
        ));
    }
    let exemplo = ler(&mut ftp, "/exemplo.txt", 0)?;
    if !exemplo.starts_with("conteúdo".as_bytes()) {
        return Err("o exemplo.txt veio diferente".to_owned());
    }

    let pasta = format!("/m31-{indice}");
    let _ = limpar(&mut ftp, &pasta);
    ftp.mkdir(&pasta)
        .map_err(|erro| format!("criar pasta: {erro}"))?;
    let arquivo = format!("{pasta}/a.bin");
    let conteudo = dados(1024 * 1024);
    gravar(&mut ftp, &arquivo, 0, &conteudo)?;
    if ler(&mut ftp, &arquivo, 0)? != conteudo {
        return Err("o download de 1 MiB veio diferente".to_owned());
    }
    let tamanho = ftp
        .stat(&arquivo)
        .map_err(|erro| format!("stat: {erro}"))?
        .size;
    if tamanho != conteudo.len() as u64 {
        return Err(format!("stat diz {tamanho} bytes"));
    }

    // Retomar o envio e o download.
    let parcial = format!("{pasta}/parcial.bin");
    let metade = conteudo.len() / 2;
    gravar(&mut ftp, &parcial, 0, &conteudo[..metade])?;
    gravar(&mut ftp, &parcial, metade as u64, &conteudo[metade..])?;
    if ler(&mut ftp, &parcial, 0)? != conteudo {
        return Err("o envio retomado ficou diferente".to_owned());
    }
    if ler(&mut ftp, &arquivo, metade as u64)? != conteudo[metade..] {
        return Err("o download retomado veio diferente".to_owned());
    }

    // Renomear.
    let novo = format!("{pasta}/b.bin");
    ftp.rename(&arquivo, &novo, false)
        .map_err(|erro| format!("renomear: {erro}"))?;
    let renomeado = ftp.stat(&novo).map_err(|erro| format!("stat: {erro}"))?;
    if renomeado.kind != EntryKind::File {
        return Err("o arquivo renomeado não é um arquivo".to_owned());
    }

    // Data.
    let alvo = UNIX_EPOCH + Duration::from_secs(1_577_836_800);
    let datas: Passo = match ftp.set_modified(&novo, alvo) {
        Ok(()) => {
            let achada = ftp
                .stat(&novo)
                .map_err(|erro| format!("stat: {erro}"))?
                .modified
                .map(|t| t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()));
            match achada {
                Some(Ok(s)) if s.abs_diff(1_577_836_800) <= 60 => Ok("ok".to_owned()),
                other => Err(format!("a data ficou {other:?}")),
            }
        }
        Err(erro) => Err(format!("recusado ({erro})")),
    };
    match datas {
        Ok(texto) => notas.push(format!("data: {texto}")),
        Err(texto) => notas.push(format!("data: {texto}")),
    }

    // Permissões.
    if ftp.caps().permissions {
        match ftp.set_permissions(&novo, 0o600) {
            Ok(()) => {
                let modo = ftp
                    .stat(&novo)
                    .map_err(|erro| format!("stat: {erro}"))?
                    .permissions;
                notas.push(format!(
                    "chmod: {}",
                    if modo.map(|m| m & 0o777) == Some(0o600) {
                        "ok".to_owned()
                    } else {
                        format!("ficou {modo:?}")
                    }
                ));
            }
            Err(erro) => notas.push(format!("chmod: recusado ({erro})")),
        }
    } else {
        notas.push("chmod: n/d".to_owned());
    }

    limpar(&mut ftp, &pasta)?;

    if caso.seguranca != FtpSecurity::None && perguntas.load(Ordering::SeqCst) == 0 {
        return Err("o certificado autoassinado não passou pela pergunta".to_owned());
    }
    if ftp.is_secure() != (caso.seguranca != FtpSecurity::None) {
        return Err("a proteção da conexão não é a pedida".to_owned());
    }
    Ok(notas)
}

fn limpar(ftp: &mut Ftp, pasta: &str) -> Result<(), String> {
    if let Ok(itens) = ftp.list(pasta) {
        for item in itens {
            ftp.remove_file(&format!("{pasta}/{}", item.name))
                .map_err(|erro| format!("apagar {}: {erro}", item.name))?;
        }
    }
    ftp.remove_dir(pasta)
        .map_err(|erro| format!("apagar a pasta: {erro}"))
}

#[test]
#[ignore = "precisa dos servidores de testes/ftp (rodar.sh)"]
fn matriz_real() {
    let casos = casos();
    let mut falhas = Vec::new();
    println!("\n{:<52} resultado", "caso");
    for (indice, caso) in casos.iter().enumerate() {
        let resultado = rodar(caso, indice);
        match &resultado {
            Ok(notas) => println!("{:<52} ✔ {}", caso.nome(), notas.join("; ")),
            Err(erro) => {
                println!("{:<52} ✘ {erro}", caso.nome());
                if caso.obrigatorio() {
                    falhas.push(format!("{}: {erro}", caso.nome()));
                }
            }
        }
    }
    assert!(
        falhas.is_empty(),
        "casos obrigatórios que falharam:\n{}",
        falhas.join("\n")
    );
}

#[test]
#[ignore = "precisa dos servidores de testes/ftp (rodar.sh)"]
fn senha_errada_e_certificado_recusado() {
    for servidor in SERVIDORES {
        let &(porta, seguranca, _) = &servidor.portas[0];
        let erro = FtpBuilder::new(HOST, porta)
            .login("teste", "errada123")
            .security(seguranca)
            .check_certificate(|_| true)
            .connect()
            .err()
            .expect("a senha errada tem de falhar");
        assert!(
            !erro.to_string().contains("errada123"),
            "{}: {erro}",
            servidor.nome
        );
        let recusado = FtpBuilder::new(HOST, porta)
            .login("teste", "teste")
            .security(seguranca)
            .check_certificate(|_| false)
            .connect();
        assert!(recusado.is_err(), "{}", servidor.nome);
    }
}
