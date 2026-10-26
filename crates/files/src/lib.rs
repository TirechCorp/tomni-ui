//! Arquivos locais e remotos do Tomni UI (M30).
//!
//! - [`Fs`]: o que um sistema de arquivos sabe fazer (bloqueante), com o [`Local`] e, na
//!   feature `sftp`, o `Sftp`; o FTP, o S3 e o rclone (`Rclone`) seguem o mesmo trait.
//! - [`Remote`]: uma conexão numa thread, que a tela comanda e lê por [`Live`] (como um signal).
//! - [`Transfers`]: a fila de envios e baixas, com paralelas, conflitos, retomar e cancelar.
//!
//! ```
//! use tomni_ui_files::{Fs, Local, path};
//!
//! let mut local = Local::new();
//! let casa = local.home().unwrap();
//! assert_eq!(path::parent(&path::join(&casa, "x")), casa);
//! ```

mod disk;
mod entry;
pub mod estimate;
mod fs;
#[cfg(feature = "ftp")]
pub mod ftp;
mod live;
mod local;
#[cfg(any(test, feature = "test-util"))]
mod mem;
pub mod path;
#[cfg(feature = "rclone")]
pub mod rclone;
mod remote;
#[cfg(feature = "s3")]
pub mod s3;
#[cfg(feature = "sftp")]
pub mod sftp;
mod transfers;

pub use disk::available_space;
pub use entry::{Caps, Entry, EntryKind};
pub use estimate::{Clock, Eta, ManualClock, SystemClock};
pub use fs::{
    ARQUIVOS_EM_VOO, Delegation, Download, Fs, Location, TreeOptions, Upload, connection_limit,
    describe, is_connection_error, is_connection_limit, unsupported,
};
pub use live::Live;
pub use local::Local;
#[cfg(any(test, feature = "test-util"))]
pub use mem::MemFs;
pub use remote::{FsError, KEEPALIVE_PADRAO, Opener, Remote, RemoteState, Reply};
pub use transfers::{
    Conflict, ConflictQuestion, Endpoint, PARALELAS_PADRAO, PRESERVAR_DATA_PADRAO,
    PRESERVAR_PERMISSOES_PADRAO, QueueState, TransferInfo, TransferKind, TransferState,
    TransferSummary, Transfers,
};
