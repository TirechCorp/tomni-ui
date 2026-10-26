//! O progresso e o número no ícone do app no dock (M49), pela interface
//! `com.canonical.Unity.LauncherEntry` do D-Bus.
//!
//! No Linux, o estado vai por um sinal `Update` no barramento de sessão, e o método `Query`
//! devolve o último estado para o dock que abrir depois do app. Fora do Linux, o
//! [`LauncherEntry`] existe e não faz nada, para o app não precisar de `cfg`.
//!
//! O dock só associa o sinal ao app se existir um arquivo `.desktop` com o `desktop_id` dado.

use std::sync::Mutex;

/// O que o dock mostra no ícone do app.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LauncherState {
    /// A barra de progresso, de `0.0` a `1.0`; `None` esconde a barra. A interface não tem o
    /// progresso indeterminado: quem só tem trabalho sem valor manda `None`.
    pub progress: Option<f64>,
    /// O número no ícone; `None` (ou `0`) esconde.
    pub count: Option<u64>,
    /// Pede atenção do dock (o ícone pula ou destaca, conforme o dock).
    pub urgent: bool,
}

impl LauncherState {
    /// O estado com o valor do progresso preso entre `0.0` e `1.0` e a contagem `0` como
    /// ausente: o que de fato vai ao dock, e o que `update` compara para só enviar mudanças.
    fn normalized(mut self) -> Self {
        self.progress = self
            .progress
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0));
        self.count = self.count.filter(|&count| count > 0);
        self
    }
}

/// O ícone do app no dock. Só envia quando o estado muda; sem barramento de sessão (ou fora do
/// Linux), não faz nada e não dá erro.
pub struct LauncherEntry {
    desktop_id: String,
    last: Mutex<Option<LauncherState>>,
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    worker: Mutex<Option<std::sync::mpsc::Sender<LauncherState>>>,
}

impl LauncherEntry {
    /// Um ícone do dock para o arquivo `desktop_id` (por exemplo `"meu-app.desktop"`).
    #[must_use]
    pub fn new(desktop_id: &str) -> Self {
        Self {
            desktop_id: desktop_id.to_owned(),
            last: Mutex::new(None),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            worker: Mutex::new(None),
        }
    }

    /// Manda o estado ao dock, se for diferente do último enviado. Sem nada para mostrar
    /// (`LauncherState::default()`), a primeira chamada não envia: o dock já está limpo.
    pub fn update(&self, state: LauncherState) {
        let state = state.normalized();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if *last == Some(state) || (last.is_none() && state == LauncherState::default()) {
            return;
        }
        *last = Some(state);
        drop(last);
        self.send(state);
    }

    #[cfg(not(all(unix, not(target_os = "macos"), not(target_os = "android"))))]
    fn send(&self, _state: LauncherState) {
        let _ = &self.desktop_id;
    }

    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    fn send(&self, state: LauncherState) {
        use std::sync::mpsc;
        use std::thread;

        let mut worker = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = worker.as_ref()
            && sender.send(state).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let desktop_id = self.desktop_id.clone();
        if tx.send(state).is_ok() {
            thread::spawn(move || linux::work(&desktop_id, &rx));
            *worker = Some(tx);
        }
    }
}

/// O `app_uri` do sinal: o dock casa com o `.desktop` do app.
#[cfg_attr(
    not(all(unix, not(target_os = "macos"), not(target_os = "android"))),
    allow(dead_code)
)]
fn app_uri(desktop_id: &str) -> String {
    format!("application://{desktop_id}")
}

/// O caminho do objeto no D-Bus, derivado do `desktop_id`: só letras, dígitos e `_` entram
/// num caminho do D-Bus, o resto vira `_`.
#[cfg_attr(
    not(all(unix, not(target_os = "macos"), not(target_os = "android"))),
    allow(dead_code)
)]
fn object_path(desktop_id: &str) -> String {
    let id: String = desktop_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("/org/tomni_ui/LauncherEntry/{id}")
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod linux {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedValue, Value};

    use super::{LauncherState, app_uri, object_path};

    const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

    /// As propriedades do sinal `Update` e da resposta do `Query`.
    pub(super) fn properties(state: &LauncherState) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        let mut put = |name: &str, value: Value<'_>| {
            if let Ok(value) = value.try_to_owned() {
                map.insert(name.to_owned(), value);
            }
        };
        put("progress", Value::F64(state.progress.unwrap_or(0.0)));
        put("progress-visible", Value::Bool(state.progress.is_some()));
        // Com o número zero (ou ausente) o dock esconde o selo; o `x` do D-Bus é com sinal.
        let count = i64::try_from(state.count.unwrap_or(0)).unwrap_or(i64::MAX);
        put("count", Value::I64(count));
        put("count-visible", Value::Bool(state.count.is_some()));
        put("urgent", Value::Bool(state.urgent));
        map
    }

    /// O objeto que responde ao `Query` com o último estado.
    struct Entry {
        app_uri: String,
        state: Arc<Mutex<LauncherState>>,
    }

    #[zbus::interface(name = "com.canonical.Unity.LauncherEntry")]
    impl Entry {
        fn query(&self) -> (String, HashMap<String, OwnedValue>) {
            let state = *self.state.lock().unwrap_or_else(|e| e.into_inner());
            (self.app_uri.clone(), properties(&state))
        }
    }

    /// A thread do ícone: conecta ao barramento de sessão, serve o `Query` e emite o `Update` a
    /// cada estado novo. Sem barramento, ela só termina.
    pub(super) fn work(desktop_id: &str, states: &mpsc::Receiver<LauncherState>) {
        let Ok(connection) = Connection::session() else {
            return;
        };
        let (uri, path) = (app_uri(desktop_id), object_path(desktop_id));
        let shared = Arc::new(Mutex::new(LauncherState::default()));
        let entry = Entry {
            app_uri: uri.clone(),
            state: Arc::clone(&shared),
        };
        if connection.object_server().at(path.as_str(), entry).is_err() {
            return;
        }
        while let Ok(state) = states.recv() {
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = state;
            let _ = connection.emit_signal(
                None::<&str>,
                path.as_str(),
                INTERFACE,
                "Update",
                &(uri.as_str(), properties(&state)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_estado_e_normalizado_antes_de_comparar() {
        let state = LauncherState {
            progress: Some(1.7),
            count: Some(0),
            urgent: false,
        }
        .normalized();
        assert_eq!(state.progress, Some(1.0));
        assert_eq!(state.count, None);
        let sem_valor = LauncherState {
            progress: Some(f64::NAN),
            ..LauncherState::default()
        }
        .normalized();
        assert_eq!(sem_valor.progress, None);
    }

    #[test]
    fn o_uri_e_o_caminho_vem_do_desktop_id() {
        assert_eq!(app_uri("meu-app.desktop"), "application://meu-app.desktop");
        assert_eq!(
            object_path("meu-app.desktop"),
            "/org/tomni_ui/LauncherEntry/meu_app_desktop"
        );
    }

    #[test]
    fn so_envia_quando_o_estado_muda() {
        let entry = LauncherEntry::new("teste.desktop");
        entry.update(LauncherState::default());
        assert_eq!(*entry.last.lock().unwrap(), None);
        let com_barra = LauncherState {
            progress: Some(0.5),
            ..LauncherState::default()
        };
        entry.update(com_barra);
        assert_eq!(*entry.last.lock().unwrap(), Some(com_barra));
        entry.update(LauncherState::default());
        assert_eq!(
            *entry.last.lock().unwrap(),
            Some(LauncherState::default()),
            "voltar ao vazio depois de mostrar algo precisa limpar o dock"
        );
    }

    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    #[test]
    fn as_propriedades_seguem_a_interface() {
        let state = LauncherState {
            progress: Some(0.25),
            count: Some(3),
            urgent: true,
        };
        let props = linux::properties(&state);
        let get = |name: &str| props.get(name).map(|v| (**v).clone());
        assert_eq!(get("progress"), Some(zbus::zvariant::Value::F64(0.25)));
        assert_eq!(
            get("progress-visible"),
            Some(zbus::zvariant::Value::Bool(true))
        );
        assert_eq!(get("count"), Some(zbus::zvariant::Value::I64(3)));
        assert_eq!(
            get("count-visible"),
            Some(zbus::zvariant::Value::Bool(true))
        );
        assert_eq!(get("urgent"), Some(zbus::zvariant::Value::Bool(true)));
        let vazio = linux::properties(&LauncherState::default());
        assert_eq!(
            vazio.get("progress-visible").map(|v| (**v).clone()),
            Some(zbus::zvariant::Value::Bool(false))
        );
    }

    /// Manual: `dbus-monitor "interface='com.canonical.Unity.LauncherEntry'"` num terminal e
    /// `cargo test -p tomni-ui-platform launcher::tests::emite_no_barramento -- --ignored`.
    #[test]
    #[ignore = "precisa de um barramento de sessão e de alguém ouvindo"]
    fn emite_no_barramento() {
        let entry = LauncherEntry::new("tomni-ui-teste.desktop");
        entry.update(LauncherState {
            progress: Some(0.4),
            count: Some(2),
            urgent: false,
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
}
