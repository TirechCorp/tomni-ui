//! Enviar uma pasta inteira pelo SFTP com vários arquivos e várias escritas em voo.
//!
//! Cada arquivo pequeno custa idas e vindas ao servidor (abrir, escrever, fechar, datas). Uma
//! de cada vez, o tempo vai quase todo em espera; aqui o disco local é percorrido uma vez, as
//! pastas são criadas um nível por vez e os arquivos seguem em `files_in_flight` tarefas, todas
//! no mesmo runtime da conexão (um só `block_on`).

use std::collections::VecDeque;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

use super::{Driver, fail, fail_io, unix_seconds};
use crate::fs::{TreeOptions, describe};
use crate::path;

/// De quanto em quanto tempo o `progress` é chamado.
const PROGRESSO_TEMPO: Duration = Duration::from_millis(100);

/// Quanto se lê do disco de cada vez.
const BLOCO: usize = 256 * 1024;

/// Um arquivo do disco a enviar.
struct Arquivo {
    local: PathBuf,
    remoto: String,
    modificado: Option<SystemTime>,
    modo: Option<u32>,
}

/// O que há no disco: as pastas por profundidade (a primeira é a de destino) e os arquivos.
struct Plano {
    pastas: Vec<Vec<String>>,
    arquivos: Vec<Arquivo>,
    total: u64,
}

/// Por que uma tarefa de arquivo acabou sem enviar.
enum Falha {
    /// O servidor recusou abrir mais um arquivo agora (`handle limit reached`, ou um `Failure`
    /// sem motivo): devolve o arquivo para a fila e usa menos em voo.
    Recusou(Arquivo, io::Error),
    /// O erro é definitivo.
    Erro(io::Error),
}

/// Percorre `from` e monta o plano. Link para arquivo vira o arquivo; link para pasta e o que
/// não é nem arquivo nem pasta ficam de fora, como na fila.
fn percorrer(from: &Path, to: &str, cancel: &AtomicBool) -> io::Result<Plano> {
    let mut plano = Plano {
        pastas: vec![vec![to.to_owned()]],
        arquivos: Vec::new(),
        total: 0,
    };
    let mut pilha = vec![(from.to_path_buf(), to.to_owned(), 0_usize)];
    while let Some((pasta, remota, nivel)) = pilha.pop() {
        if cancel.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let nome_pasta = pasta.to_string_lossy().into_owned();
        let mut itens = std::fs::read_dir(&pasta)
            .and_then(|itens| itens.collect::<io::Result<Vec<_>>>())
            .map_err(|error| describe(error, "ler", &nome_pasta))?;
        itens.sort_by_key(|item| item.file_name());
        for item in itens {
            let nome = item.file_name().to_string_lossy().into_owned();
            let local = item.path();
            let destino = path::join(&remota, &nome);
            let tipo = item
                .file_type()
                .map_err(|error| describe(error, "ler", &local.to_string_lossy()))?;
            let dados = if tipo.is_dir() {
                if plano.pastas.len() <= nivel + 1 {
                    plano.pastas.push(Vec::new());
                }
                plano.pastas[nivel + 1].push(destino.clone());
                pilha.push((local, destino, nivel + 1));
                continue;
            } else if tipo.is_file() {
                item.metadata()
            } else if tipo.is_symlink() {
                match std::fs::metadata(&local) {
                    Ok(dados) if dados.is_file() => Ok(dados),
                    _ => continue,
                }
            } else {
                continue;
            };
            let dados = dados.map_err(|error| describe(error, "ler", &local.to_string_lossy()))?;
            plano.total += dados.len();
            plano.arquivos.push(Arquivo {
                local,
                remoto: destino,
                modificado: dados.modified().ok(),
                modo: modo(&dados),
            });
        }
    }
    Ok(plano)
}

#[cfg(unix)]
fn modo(dados: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(dados.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn modo(_: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Envia a pasta `from` para `to` (que não existe ainda). `progress` recebe `(feito, total)` em
/// bytes. Se `cancel` virar `true`, para e devolve `Interrupted`; o que já foi enviado fica.
pub(super) fn enviar(
    session: &Arc<SftpSession>,
    driver: &Driver,
    from: &Path,
    to: &str,
    options: &TreeOptions,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !driver.is_open() {
        return Err(describe(
            io::ErrorKind::ConnectionReset.into(),
            "gravar em",
            to,
        ));
    }
    let plano = percorrer(from, to, cancel)?;
    progress(0, plano.total);
    driver.block_on(rodar(session, plano, options, progress, cancel))
}

async fn rodar(
    session: &Arc<SftpSession>,
    plano: Plano,
    options: &TreeOptions,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> io::Result<()> {
    let Plano {
        pastas,
        arquivos,
        total,
    } = plano;
    let parar = Arc::new(AtomicBool::new(false));
    let feito = Arc::new(AtomicU64::new(0));
    let em_voo = options.files_in_flight.max(1);

    // As pastas, um nível por vez: a de baixo só existe depois da de cima.
    for (nivel, pastas) in pastas.into_iter().enumerate() {
        let mut fila: VecDeque<String> = pastas.into();
        let mut criando = JoinSet::new();
        let mut erro = None;
        loop {
            while criando.len() < em_voo
                && erro.is_none()
                && let Some(pasta) = fila.pop_front()
            {
                let session = Arc::clone(session);
                criando.spawn(async move {
                    // O nível 0 é o destino, que o chamador garantiu que não existe.
                    session
                        .create_dir(pasta.clone())
                        .await
                        .map_err(|error| fail(&error, "criar", &pasta))
                });
            }
            let Some(juntou) = criando.join_next().await else {
                break;
            };
            let resultado = juntou.map_err(io::Error::other).and_then(|r| r);
            if let Err(error) = resultado
                && erro.is_none()
            {
                erro = Some(error);
            }
            if cancel.load(Ordering::SeqCst) && erro.is_none() {
                erro = Some(io::ErrorKind::Interrupted.into());
            }
        }
        if let Some(error) = erro {
            return Err(error);
        }
        let _ = nivel;
    }

    let mut pendentes: VecDeque<Arquivo> = arquivos.into();
    let mut limite = em_voo;
    let mut tarefas: JoinSet<Result<(), Falha>> = JoinSet::new();
    let mut relogio = tokio::time::interval(PROGRESSO_TEMPO);
    relogio.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut primeiro_erro: Option<io::Error> = None;
    loop {
        while tarefas.len() < limite
            && primeiro_erro.is_none()
            && let Some(arquivo) = pendentes.pop_front()
        {
            tarefas.spawn(enviar_arquivo(
                Arc::clone(session),
                arquivo,
                options.preserve_date,
                options.preserve_permissions,
                Arc::clone(&parar),
                Arc::clone(&feito),
            ));
        }
        if tarefas.is_empty() {
            break;
        }
        tokio::select! {
            juntou = tarefas.join_next() => {
                match juntou {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(Falha::Recusou(arquivo, error)))) => {
                        let abertos = tarefas.len() + 1;
                        if abertos > 1 {
                            // Usa menos em voo e tenta de novo este arquivo.
                            limite = limite.min((abertos / 2).max(1));
                            pendentes.push_front(arquivo);
                        } else if primeiro_erro.is_none() {
                            primeiro_erro = Some(error);
                        }
                    }
                    Some(Ok(Err(Falha::Erro(error)))) => {
                        // As outras tarefas veem `parar` no próximo bloco; guarda o erro de
                        // quem falhou primeiro (as demais só dizem "interrompido").
                        if primeiro_erro.is_none() {
                            primeiro_erro = Some(error);
                        }
                    }
                    Some(Err(error)) => {
                        if primeiro_erro.is_none() {
                            primeiro_erro = Some(io::Error::other(error));
                        }
                    }
                }
                if primeiro_erro.is_some() {
                    parar.store(true, Ordering::SeqCst);
                }
            }
            _ = relogio.tick() => {
                progress(feito.load(Ordering::SeqCst).min(total), total);
                if cancel.load(Ordering::SeqCst) {
                    parar.store(true, Ordering::SeqCst);
                    if primeiro_erro.is_none() {
                        primeiro_erro = Some(io::ErrorKind::Interrupted.into());
                    }
                }
            }
        }
    }
    if let Some(error) = primeiro_erro {
        return Err(error);
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    progress(total, total);
    Ok(())
}

/// Envia um arquivo: abre, escreve (com várias escritas em voo, pela configuração da sessão),
/// fecha e põe a data e as permissões.
async fn enviar_arquivo(
    session: Arc<SftpSession>,
    arquivo: Arquivo,
    data: bool,
    permissoes: bool,
    parar: Arc<AtomicBool>,
    feito: Arc<AtomicU64>,
) -> Result<(), Falha> {
    let interrompido = || Falha::Erro(io::ErrorKind::Interrupted.into());
    if parar.load(Ordering::SeqCst) {
        return Err(interrompido());
    }
    let nome_local = arquivo.local.to_string_lossy().into_owned();
    let mut origem = std::fs::File::open(&arquivo.local)
        .map_err(|error| Falha::Erro(describe(error, "ler", &nome_local)))?;
    let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
    let mut destino = match session.open_with_flags(arquivo.remoto.clone(), flags).await {
        Ok(destino) => destino,
        Err(error) => {
            let recusa = matches!(
                &error,
                SftpError::Limited(_)
                    | SftpError::Status(russh_sftp::protocol::Status {
                        status_code: StatusCode::Failure,
                        ..
                    })
            );
            let error = fail(&error, "gravar em", &arquivo.remoto);
            return Err(if recusa {
                Falha::Recusou(arquivo, error)
            } else {
                Falha::Erro(error)
            });
        }
    };
    let mut bloco = vec![0_u8; BLOCO];
    loop {
        if parar.load(Ordering::SeqCst) {
            // Espera as escritas que já saíram, para que nada continue chegando depois.
            let _ = destino.shutdown().await;
            return Err(interrompido());
        }
        let lidos = origem
            .read(&mut bloco)
            .map_err(|error| Falha::Erro(describe(error, "ler", &nome_local)))?;
        if lidos == 0 {
            break;
        }
        destino
            .write_all(&bloco[..lidos])
            .await
            .map_err(|error| Falha::Erro(fail_io(error, "gravar em", &arquivo.remoto)))?;
        feito.fetch_add(lidos as u64, Ordering::SeqCst);
    }
    destino
        .shutdown()
        .await
        .map_err(|error| Falha::Erro(fail_io(error, "gravar em", &arquivo.remoto)))?;
    let atributos = FileAttributes {
        permissions: arquivo.modo.filter(|_| permissoes),
        atime: arquivo.modificado.filter(|_| data).map(unix_seconds),
        mtime: arquivo.modificado.filter(|_| data).map(unix_seconds),
        ..FileAttributes::empty()
    };
    if atributos.permissions.is_some() || atributos.mtime.is_some() {
        // Como na fila, a data e as permissões que o servidor não aceita não derrubam o envio.
        let _ = session
            .set_metadata(arquivo.remoto.clone(), atributos)
            .await;
    }
    Ok(())
}
