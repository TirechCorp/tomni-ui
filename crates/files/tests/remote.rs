//! A `Remote`: a thread da conexão, o estado, as respostas e a reconexão.

mod comum;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tomni_ui_files::{Fs, MemFs, Opener, Remote, RemoteState};

use comum::esperar;

fn esperar_estado(remoto: &Remote, pronto: impl Fn(&RemoteState) -> bool) {
    let limite = Instant::now() + Duration::from_secs(10);
    while !pronto(&remoto.state().get_untracked()) {
        assert!(
            Instant::now() < limite,
            "estado: {:?}",
            remoto.state().get_untracked()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn conecta_lista_e_conta_o_que_sabe_fazer() {
    let mem = MemFs::new();
    mem.put("/home/ana/a.txt", b"a").dir("/home/ana/pasta");
    let remoto = Remote::spawn("teste", mem.opener());
    esperar_estado(&remoto, |s| *s == RemoteState::Ready);
    assert!(remoto.caps().is_some());

    let casa = esperar(&remoto.home()).unwrap();
    assert_eq!(casa, "/home/ana");
    let mut nomes: Vec<String> = esperar(&remoto.list(&casa))
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    nomes.sort();
    assert_eq!(nomes, ["a.txt", "pasta"]);

    esperar(&remoto.mkdir("/home/ana/nova")).unwrap();
    esperar(&remoto.rename("/home/ana/a.txt", "/home/ana/nova/b.txt", false)).unwrap();
    assert!(mem.exists("/home/ana/nova/b.txt"));
    esperar(&remoto.remove_file("/home/ana/nova/b.txt")).unwrap();
    assert!(!mem.exists("/home/ana/nova/b.txt"));
}

#[test]
fn o_erro_do_pedido_chega_em_portugues_e_a_conexao_segue_boa() {
    let mem = MemFs::new();
    let remoto = Remote::spawn("erro", mem.opener());
    let erro = esperar(&remoto.stat("/nada")).unwrap_err();
    assert!(erro.contains("não encontrado"), "{erro}");
    assert_eq!(remoto.state().get_untracked(), RemoteState::Ready);
}

#[test]
fn falha_ao_conectar_vira_failed_e_reconnect_tenta_de_novo() {
    let mem = MemFs::new();
    let tentativas = Arc::new(AtomicUsize::new(0));
    let opener: Opener = {
        let mem_opener = mem.opener();
        let tentativas = Arc::clone(&tentativas);
        Arc::new(move || {
            if tentativas.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            } else {
                mem_opener()
            }
        })
    };
    let remoto = Remote::spawn("recusa", opener);
    esperar_estado(&remoto, |s| matches!(s, RemoteState::Failed(_)));
    assert_eq!(
        remoto.state().get_untracked(),
        RemoteState::Failed("conexão recusada".to_owned())
    );
    remoto.reconnect();
    esperar_estado(&remoto, |s| *s == RemoteState::Ready);
    assert!(esperar(&remoto.home()).is_ok());
}

#[test]
fn o_memfs_conta_as_listagens_de_todas_as_conexoes() {
    let mem = MemFs::new();
    mem.dir("/home/ana/sub");
    let um = Remote::spawn("conta-1", mem.opener());
    let outro = Remote::spawn("conta-2", mem.opener());
    assert_eq!(mem.lists(), 0);
    esperar(&um.list("/home/ana")).expect("listou");
    esperar(&outro.list("/home/ana/sub")).expect("listou");
    esperar(&um.stat("/home/ana")).expect("stat não lista");
    assert_eq!(mem.lists(), 2);
}

#[test]
fn conexao_derrubada_reabre_uma_vez_e_repete_o_pedido() {
    let mem = MemFs::new();
    mem.put("/home/ana/a.txt", b"a");
    let remoto = Remote::spawn("queda", mem.opener());
    assert!(esperar(&remoto.list("/home/ana")).is_ok());
    let antes = mem.connections();

    mem.drop_connections();
    let lista = esperar(&remoto.list("/home/ana")).expect("reabriu e repetiu");
    assert_eq!(lista.len(), 1);
    assert_eq!(mem.connections(), antes + 1);
    assert_eq!(remoto.state().get_untracked(), RemoteState::Ready);
}

#[test]
fn fechar_leva_ao_estado_closed_e_os_pedidos_novos_falham() {
    let mem = MemFs::new();
    let remoto = Remote::spawn("fecha", mem.opener());
    esperar(&remoto.home()).unwrap();
    let estado = remoto.state();
    remoto.close();
    esperar_estado(&remoto, |s| *s == RemoteState::Closed);
    let erro = esperar(&remoto.home()).unwrap_err();
    assert!(erro.contains("fechada"), "{erro}");
    assert_eq!(estado.get_untracked(), RemoteState::Closed);
}

#[test]
fn soltar_a_ultima_copia_fecha_a_conexao() {
    let mem = MemFs::new();
    let estado = {
        let remoto = Remote::spawn("solta", mem.opener());
        esperar(&remoto.home()).unwrap();
        remoto.state()
    };
    let limite = Instant::now() + Duration::from_secs(10);
    while estado.get_untracked() != RemoteState::Closed {
        assert!(Instant::now() < limite, "a conexão não fechou");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn parada_demais_manda_keepalive_e_a_queda_aparece() {
    struct Mudo(Box<dyn Fs>, Arc<AtomicUsize>);
    // Delega tudo ao MemFs; só o keepalive conta e falha na segunda vez.
    macro_rules! delega {
        ($($nome:ident($($arg:ident: $tipo:ty),*) -> $ret:ty;)*) => {
            $(fn $nome(&mut self, $($arg: $tipo),*) -> $ret { self.0.$nome($($arg),*) })*
        };
    }
    impl Fs for Mudo {
        fn caps(&self) -> tomni_ui_files::Caps {
            self.0.caps()
        }
        delega! {
            home() -> std::io::Result<String>;
            list(dir: &str) -> std::io::Result<Vec<tomni_ui_files::Entry>>;
            stat(path: &str) -> std::io::Result<tomni_ui_files::Entry>;
            lstat(path: &str) -> std::io::Result<tomni_ui_files::Entry>;
            mkdir(path: &str) -> std::io::Result<()>;
            remove_file(path: &str) -> std::io::Result<()>;
            remove_dir(path: &str) -> std::io::Result<()>;
            rename(from: &str, to: &str, overwrite: bool) -> std::io::Result<()>;
            set_permissions(path: &str, mode: u32) -> std::io::Result<()>;
            set_modified(path: &str, time: std::time::SystemTime) -> std::io::Result<()>;
            symlink(target: &str, link: &str) -> std::io::Result<()>;
            read_link(path: &str) -> std::io::Result<String>;
        }
        fn open_read(
            &mut self,
            path: &str,
            offset: u64,
        ) -> std::io::Result<Box<dyn tomni_ui_files::Download + '_>> {
            self.0.open_read(path, offset)
        }
        fn open_write(
            &mut self,
            path: &str,
            offset: u64,
        ) -> std::io::Result<Box<dyn tomni_ui_files::Upload + '_>> {
            self.0.open_write(path, offset)
        }
        fn keepalive(&mut self) -> std::io::Result<()> {
            if self.1.fetch_add(1, Ordering::SeqCst) >= 1 {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            } else {
                Ok(())
            }
        }
    }

    let mem = MemFs::new();
    let contador = Arc::new(AtomicUsize::new(0));
    let opener: Opener = {
        let mem_opener = mem.opener();
        let contador = Arc::clone(&contador);
        Arc::new(move || Ok(Box::new(Mudo(mem_opener()?, Arc::clone(&contador))) as Box<dyn Fs>))
    };
    let remoto = Remote::spawn_with("vivo", opener, Duration::from_millis(30));
    esperar_estado(&remoto, |s| matches!(s, RemoteState::Failed(_)));
    assert!(contador.load(Ordering::SeqCst) >= 2);
}
