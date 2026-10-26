//! Seletor de arquivo do sistema.
//!
//! ```ignore
//! dialogs::open_file()
//!     .title("Chave privada")
//!     .filter("Chaves", &["pem", "key"])
//!     .show(move |caminho| {
//!         if let Some(caminho) = caminho {
//!             chave.set(caminho.display().to_string());
//!         }
//!     });
//! ```
//!
//! O callback roda na thread da interface, **no frame seguinte** ao pedido, nunca dentro do
//! handler que pediu (o widget que tratava o evento ainda estaria emprestado). A resposta é
//! `None` quando a pessoa cancela, quando [`is_supported`] é `false` (Android) e quando não há
//! serviço instalado (ferramentas, lint, `Ui` sem janela). Nos testes, o `TestApp` instala um
//! serviço falso e responde com `file_dialog_answer`.
//!
//! Vários arquivos de uma vez: [`open_files`] e [`FileDialog::show_many`].
//!
//! Com [`FileDialog::remember`], o diálogo abre na pasta da última resposta com o mesmo nome
//! (como o `SettingsIdentifier` do `FileOpenPicker` do WinUI), e não no `.directory(..)`. A
//! lib guarda as pastas na memória; o app que quer lembrá-las entre execuções instala um
//! [`FolderMemory`] com [`set_folder_memory`].
//!
//! Quem mostra o diálogo é um [`FileDialogService`]: a plataforma instala o de verdade ao abrir
//! a janela. O estado (serviço, callbacks e respostas) é por thread, então testes em paralelo
//! não trocam respostas entre si.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

/// Um filtro da lista de tipos do diálogo: um nome e as extensões, sem o ponto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    /// O nome mostrado na lista de tipos.
    pub name: String,
    /// As extensões, sem o ponto.
    pub extensions: Vec<String>,
}

/// Abrir um ou vários arquivos que existem, escolher onde salvar ou escolher uma pasta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogKind {
    /// Abrir um arquivo que existe.
    Open,
    /// Abrir um ou mais arquivos que existem.
    OpenMany,
    /// Escolher onde salvar.
    Save,
    /// Escolher uma pasta que existe.
    Folder,
}

/// O que o app pediu: o que o serviço precisa para mostrar o diálogo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogRequest {
    /// Abrir ou salvar.
    pub kind: FileDialogKind,
    /// O título da janela.
    pub title: String,
    /// Os filtros, na ordem em que o app declarou.
    pub filters: Vec<FileFilter>,
    /// A pasta onde o diálogo abre.
    pub directory: Option<PathBuf>,
    /// O nome sugerido, ao salvar.
    pub file_name: Option<String>,
}

/// Quem mostra o diálogo. Responda com [`FileDialogAnswer::send`], de qualquer thread.
pub trait FileDialogService {
    /// Mostra o diálogo para `request` e, quando a pessoa responder, chama `answer.send`.
    fn show(&self, request: FileDialogRequest, answer: FileDialogAnswer);
}

/// Onde ficam as pastas de [`FileDialog::remember`] entre uma execução e outra. Instale com
/// [`set_folder_memory`].
pub trait FolderMemory {
    /// A última pasta guardada com este nome.
    fn folder(&self, key: &str) -> Option<PathBuf>;
    /// Guarda a pasta da resposta mais recente com este nome.
    fn set_folder(&self, key: &str, folder: &Path);
}

type Answered = (u64, Vec<PathBuf>);

/// Por onde o serviço devolve a resposta. É `Send`: o diálogo pode rodar em outra thread.
/// Descartar sem responder equivale a responder `None`.
pub struct FileDialogAnswer {
    id: u64,
    tx: mpsc::Sender<Answered>,
    sent: bool,
}

impl FileDialogAnswer {
    /// Entrega a resposta (`None` se a pessoa cancelou) e acorda o loop de eventos.
    pub fn send(self, path: Option<PathBuf>) {
        self.send_many(path.into_iter().collect());
    }

    /// Entrega os arquivos escolhidos (vazio se a pessoa cancelou), para [`open_files`].
    pub fn send_many(mut self, paths: Vec<PathBuf>) {
        self.sent = true;
        let _ = self.tx.send((self.id, paths));
        crate::wake();
    }
}

impl Drop for FileDialogAnswer {
    fn drop(&mut self) {
        if !self.sent {
            let _ = self.tx.send((self.id, Vec::new()));
            crate::wake();
        }
    }
}

type Callback = Box<dyn FnOnce(Vec<PathBuf>)>;

/// Um pedido sem resposta: quem chamar e onde guardar a pasta.
struct Pending {
    callback: Callback,
    kind: FileDialogKind,
    remember: Option<String>,
}

struct State {
    service: Option<Rc<dyn FileDialogService>>,
    memory: Option<Rc<dyn FolderMemory>>,
    /// As pastas de `remember` sem um [`FolderMemory`] instalado.
    folders: HashMap<String, PathBuf>,
    next: u64,
    pending: HashMap<u64, Pending>,
    tx: mpsc::Sender<Answered>,
    rx: mpsc::Receiver<Answered>,
    ready: VecDeque<Answered>,
}

impl State {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            service: None,
            memory: None,
            folders: HashMap::new(),
            next: 0,
            pending: HashMap::new(),
            tx,
            rx,
            ready: VecDeque::new(),
        }
    }

    fn collect(&mut self) {
        while let Ok(answered) = self.rx.try_recv() {
            self.ready.push_back(answered);
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

/// Se a plataforma sabe mostrar um seletor de arquivo. No Android é `false`: o pedido é
/// respondido com `None` e o app deve deixar digitar o caminho.
pub fn is_supported() -> bool {
    cfg!(not(target_os = "android"))
}

/// Instala quem mostra os diálogos desta thread. A plataforma chama ao abrir a janela; o
/// `TestApp`, com um falso.
#[doc(hidden)]
pub fn set_service(service: Rc<dyn FileDialogService>) {
    STATE.with(|s| s.borrow_mut().service = Some(service));
}

/// Instala onde as pastas de [`FileDialog::remember`] ficam guardadas, nesta thread. Sem ele,
/// ficam na memória até o app fechar.
pub fn set_folder_memory(memory: Rc<dyn FolderMemory>) {
    STATE.with(|s| s.borrow_mut().memory = Some(memory));
}

/// A pasta guardada com este nome, se ainda existe.
fn remembered(key: &str) -> Option<PathBuf> {
    let (memory, local) = STATE.with(|s| {
        let s = s.borrow();
        (s.memory.clone(), s.folders.get(key).cloned())
    });
    memory
        .map_or(local, |m| m.folder(key))
        .filter(|pasta| pasta.is_dir())
}

/// Guarda a pasta da resposta: a própria, no seletor de pasta; a de cima, nos outros.
fn remember(key: &str, kind: FileDialogKind, path: &Path) {
    let folder = match kind {
        FileDialogKind::Folder => Some(path),
        _ => path.parent(),
    };
    let Some(folder) = folder.filter(|f| !f.as_os_str().is_empty()) else {
        return;
    };
    let memory = STATE.with(|s| s.borrow().memory.clone());
    match memory {
        Some(memory) => memory.set_folder(key, folder),
        None => STATE.with(|s| {
            s.borrow_mut()
                .folders
                .insert(key.to_owned(), folder.to_owned());
        }),
    }
}

/// Tira o que [`set_service`] instalou.
#[doc(hidden)]
pub fn clear_service() {
    STATE.with(|s| s.borrow_mut().service = None);
}

/// Roda os callbacks das respostas que chegaram. O `Ui::frame` chama no começo. Retorna se
/// algum rodou.
#[doc(hidden)]
pub fn deliver() -> bool {
    let ready = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.collect();
        std::mem::take(&mut s.ready)
    });
    let mut any = false;
    for (id, paths) in ready {
        let pending = STATE.with(|s| s.borrow_mut().pending.remove(&id));
        if let Some(pending) = pending {
            if let (Some(key), Some(first)) = (&pending.remember, paths.first()) {
                remember(key, pending.kind, first);
            }
            (pending.callback)(paths);
            any = true;
        }
    }
    any
}

/// Se há resposta esperando para o próximo [`deliver`]. O `Ui::deadline` usa para a plataforma
/// rodar o frame.
#[doc(hidden)]
pub fn has_answer() -> bool {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.collect();
        !s.ready.is_empty()
    })
}

/// Pede um arquivo que existe.
pub fn open_file() -> FileDialog {
    FileDialog::new(FileDialogKind::Open)
}

/// Pede um ou mais arquivos que existem; responda com [`FileDialog::show_many`].
pub fn open_files() -> FileDialog {
    FileDialog::new(FileDialogKind::OpenMany)
}

/// Pede onde salvar um arquivo.
pub fn save_file() -> FileDialog {
    FileDialog::new(FileDialogKind::Save)
}

/// Pede uma pasta que existe. Os filtros e o nome sugerido não valem; `.directory(..)` diz onde
/// o diálogo abre.
pub fn pick_folder() -> FileDialog {
    FileDialog::new(FileDialogKind::Folder)
}

/// O pedido em construção; termina em [`FileDialog::show`].
pub struct FileDialog {
    request: FileDialogRequest,
    remember: Option<String>,
}

impl FileDialog {
    fn new(kind: FileDialogKind) -> Self {
        Self {
            request: FileDialogRequest {
                kind,
                title: String::new(),
                filters: Vec::new(),
                directory: None,
                file_name: None,
            },
            remember: None,
        }
    }

    /// O título da janela do diálogo.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.request.title = title.into();
        self
    }

    /// Acrescenta um filtro: `.filter("Imagens", &["png", "jpg"])`. Extensões sem o ponto.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.request.filters.push(FileFilter {
            name: name.into(),
            extensions: extensions.iter().map(|e| (*e).to_string()).collect(),
        });
        self
    }

    /// A pasta onde o diálogo abre.
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.request.directory = Some(directory.into());
        self
    }

    /// Lembra a pasta da resposta com este nome e abre nela da próxima vez, no lugar de
    /// `.directory(..)` (que vale enquanto não há pasta lembrada, ou se ela sumiu). Diálogos que
    /// servem à mesma coisa podem dividir o nome.
    pub fn remember(mut self, key: impl Into<String>) -> Self {
        self.remember = Some(key.into());
        self
    }

    /// O nome sugerido, em [`save_file`].
    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.request.file_name = Some(name.into());
        self
    }

    /// Mostra o diálogo. `on_answer` roda na thread da interface, no frame seguinte, com o
    /// caminho escolhido ou `None` (em [`open_files`], o primeiro dos escolhidos).
    pub fn show(self, on_answer: impl FnOnce(Option<PathBuf>) + 'static) {
        self.show_many(move |paths| on_answer(paths.into_iter().next()));
    }

    /// Mostra o diálogo e responde com todos os arquivos escolhidos, vazio se a pessoa cancelou.
    /// Com [`open_files`] podem ser vários; com os outros, no máximo um.
    pub fn show_many(mut self, on_answer: impl FnOnce(Vec<PathBuf>) + 'static) {
        if let Some(folder) = self.remember.as_deref().and_then(remembered) {
            self.request.directory = Some(folder);
        }
        let pending = Pending {
            callback: Box::new(on_answer),
            kind: self.request.kind,
            remember: self.remember,
        };
        let (answer, service) = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let id = s.next;
            s.next += 1;
            s.pending.insert(id, pending);
            let answer = FileDialogAnswer {
                id,
                tx: s.tx.clone(),
                sent: false,
            };
            (answer, s.service.clone())
        });
        match service {
            Some(service) if is_supported() => service.show(self.request, answer),
            // Sem serviço (ou sem suporte): `None` pela mesma fila, entregue no `deliver`.
            _ => answer.send_many(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Falso {
        pedidos: RefCell<Vec<FileDialogRequest>>,
        respostas: RefCell<Vec<FileDialogAnswer>>,
    }

    impl FileDialogService for Falso {
        fn show(&self, request: FileDialogRequest, answer: FileDialogAnswer) {
            self.pedidos.borrow_mut().push(request);
            self.respostas.borrow_mut().push(answer);
        }
    }

    fn falso() -> Rc<Falso> {
        let falso = Rc::new(Falso {
            pedidos: RefCell::new(Vec::new()),
            respostas: RefCell::new(Vec::new()),
        });
        set_service(falso.clone());
        falso
    }

    #[test]
    fn sem_servico_responde_none_so_no_deliver() {
        clear_service();
        let resposta = Rc::new(RefCell::new(None::<Option<PathBuf>>));
        let r = resposta.clone();
        open_file().show(move |p| *r.borrow_mut() = Some(p));
        assert!(resposta.borrow().is_none(), "nunca síncrono");
        assert!(has_answer());
        assert!(deliver());
        assert_eq!(*resposta.borrow(), Some(None));
        assert!(!has_answer());
        assert!(!deliver());
    }

    #[test]
    fn resposta_de_outra_thread() {
        let falso = falso();
        let resposta = Rc::new(RefCell::new(None::<Option<PathBuf>>));
        let r = resposta.clone();
        save_file()
            .title("Salvar")
            .filter("Texto", &["txt"])
            .directory("/tmp")
            .file_name("a.txt")
            .show(move |p| *r.borrow_mut() = Some(p));
        let pedido = falso.pedidos.borrow()[0].clone();
        assert_eq!(pedido.kind, FileDialogKind::Save);
        assert_eq!(pedido.title, "Salvar");
        assert_eq!(pedido.filters[0].extensions, ["txt"]);
        assert_eq!(pedido.directory, Some(PathBuf::from("/tmp")));
        assert_eq!(pedido.file_name.as_deref(), Some("a.txt"));
        assert!(!has_answer());
        let answer = falso.respostas.borrow_mut().pop().unwrap();
        std::thread::spawn(move || answer.send(Some(PathBuf::from("/tmp/a.txt"))))
            .join()
            .unwrap();
        assert!(has_answer());
        deliver();
        assert_eq!(*resposta.borrow(), Some(Some(PathBuf::from("/tmp/a.txt"))));
        clear_service();
    }

    #[test]
    fn pick_folder_pede_uma_pasta() {
        let falso = falso();
        let resposta = Rc::new(RefCell::new(None::<Option<PathBuf>>));
        let r = resposta.clone();
        pick_folder()
            .title("Pasta de dados")
            .directory("/tmp")
            .show(move |p| *r.borrow_mut() = Some(p));
        let pedido = falso.pedidos.borrow()[0].clone();
        assert_eq!(pedido.kind, FileDialogKind::Folder);
        assert_eq!(pedido.title, "Pasta de dados");
        assert_eq!(pedido.directory, Some(PathBuf::from("/tmp")));
        let answer = falso.respostas.borrow_mut().pop().unwrap();
        answer.send(Some(PathBuf::from("/tmp/dados")));
        deliver();
        assert_eq!(*resposta.borrow(), Some(Some(PathBuf::from("/tmp/dados"))));
        clear_service();
    }

    #[test]
    fn respostas_fora_de_ordem_chamam_o_callback_certo() {
        let falso = falso();
        let log = Rc::new(RefCell::new(Vec::<(u8, Option<PathBuf>)>::new()));
        for n in 0..2u8 {
            let l = log.clone();
            open_file().show(move |p| l.borrow_mut().push((n, p)));
        }
        let segunda = falso.respostas.borrow_mut().pop().unwrap();
        let primeira = falso.respostas.borrow_mut().pop().unwrap();
        segunda.send(Some("b".into()));
        deliver();
        primeira.send(None);
        deliver();
        assert_eq!(*log.borrow(), [(1, Some("b".into())), (0, None)]);
        clear_service();
    }

    #[test]
    fn descartar_a_resposta_equivale_a_none() {
        let falso = falso();
        let chamado = Rc::new(Cell::new(false));
        let c = chamado.clone();
        open_file().show(move |p| {
            assert!(p.is_none());
            c.set(true);
        });
        drop(falso.respostas.borrow_mut().pop());
        deliver();
        assert!(chamado.get());
        clear_service();
    }

    #[test]
    fn callback_pode_pedir_outro_dialogo() {
        clear_service();
        let n = Rc::new(Cell::new(0));
        let n2 = n.clone();
        open_file().show(move |_| {
            n2.set(n2.get() + 1);
            let n3 = n2.clone();
            open_file().show(move |_| n3.set(n3.get() + 10));
        });
        deliver();
        assert_eq!(n.get(), 1, "o segundo só no frame seguinte");
        deliver();
        assert_eq!(n.get(), 11);
    }

    #[test]
    fn open_files_entrega_todos_e_show_so_o_primeiro() {
        let falso = falso();
        let todos = Rc::new(RefCell::new(Vec::new()));
        let t = todos.clone();
        open_files().show_many(move |p| *t.borrow_mut() = p);
        assert_eq!(falso.pedidos.borrow()[0].kind, FileDialogKind::OpenMany);
        let resposta = falso.respostas.borrow_mut().pop().unwrap();
        resposta.send_many(vec!["/a/1".into(), "/a/2".into()]);
        deliver();
        assert_eq!(
            *todos.borrow(),
            [PathBuf::from("/a/1"), PathBuf::from("/a/2")]
        );

        let primeiro = Rc::new(RefCell::new(None));
        let p1 = primeiro.clone();
        open_files().show(move |p| *p1.borrow_mut() = Some(p));
        let resposta = falso.respostas.borrow_mut().pop().unwrap();
        resposta.send_many(vec!["/b/1".into(), "/b/2".into()]);
        deliver();
        assert_eq!(*primeiro.borrow(), Some(Some(PathBuf::from("/b/1"))));
        clear_service();
    }

    /// Uma pasta de verdade para o teste, que some no fim.
    struct Pasta(PathBuf);

    impl Pasta {
        fn nova(nome: &str) -> Self {
            let pasta =
                std::env::temp_dir().join(format!("tomni-dialogs-{nome}-{}", std::process::id()));
            std::fs::create_dir_all(pasta.join("dentro")).unwrap();
            Self(pasta)
        }
    }

    impl Drop for Pasta {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn responder(falso: &Falso, caminho: PathBuf) {
        let resposta = falso.respostas.borrow_mut().pop().unwrap();
        resposta.send(Some(caminho));
        deliver();
    }

    #[test]
    fn remember_abre_na_pasta_da_ultima_resposta() {
        let falso = falso();
        let pasta = Pasta::nova("lembra");
        open_file().remember("chaves").directory("/x").show(|_| {});
        assert_eq!(falso.pedidos.borrow()[0].directory, Some("/x".into()));
        responder(&falso, pasta.0.join("dentro/id_ed25519"));
        open_file().remember("chaves").directory("/x").show(|_| {});
        assert_eq!(
            falso.pedidos.borrow()[1].directory,
            Some(pasta.0.join("dentro"))
        );
        // Outro nome não herda; o seletor de pasta guarda a própria pasta.
        pick_folder().remember("outra").show(|_| {});
        assert_eq!(falso.pedidos.borrow()[2].directory, None);
        responder(&falso, pasta.0.clone());
        save_file().remember("outra").show(|_| {});
        assert_eq!(falso.pedidos.borrow()[3].directory, Some(pasta.0.clone()));
        // Cancelar não esquece, e a pasta que sumiu volta ao `.directory(..)`.
        falso.respostas.borrow_mut().pop().unwrap().send(None);
        deliver();
        std::fs::remove_dir_all(pasta.0.join("dentro")).unwrap();
        open_file().remember("chaves").directory("/x").show(|_| {});
        assert_eq!(falso.pedidos.borrow()[4].directory, Some("/x".into()));
        clear_service();
    }

    #[test]
    fn remember_usa_a_memoria_instalada() {
        #[derive(Default)]
        struct Memoria(RefCell<HashMap<String, PathBuf>>);
        impl FolderMemory for Memoria {
            fn folder(&self, key: &str) -> Option<PathBuf> {
                self.0.borrow().get(key).cloned()
            }
            fn set_folder(&self, key: &str, folder: &Path) {
                self.0
                    .borrow_mut()
                    .insert(key.to_owned(), folder.to_owned());
            }
        }
        let falso = falso();
        let pasta = Pasta::nova("memoria");
        let memoria = Rc::new(Memoria::default());
        memoria
            .0
            .borrow_mut()
            .insert("chaves".into(), pasta.0.clone());
        set_folder_memory(memoria.clone());
        open_files().remember("chaves").show_many(|_| {});
        assert_eq!(falso.pedidos.borrow()[0].directory, Some(pasta.0.clone()));
        let resposta = falso.respostas.borrow_mut().pop().unwrap();
        resposta.send_many(vec![pasta.0.join("dentro/a"), pasta.0.join("b")]);
        deliver();
        assert_eq!(memoria.folder("chaves"), Some(pasta.0.join("dentro")));
        STATE.with(|s| s.borrow_mut().memory = None);
        clear_service();
    }
}
