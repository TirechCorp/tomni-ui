//! Os critérios do spike do M1, mantidos como testes de regressão do runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::prelude::*;
use crate::{init, live_tasks};

type Log = Rc<RefCell<Vec<String>>>;

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut log.borrow_mut())
}

fn logged(log: &Log, name: &'static str, f: impl Fn() -> String + 'static) {
    let log = Rc::clone(log);
    effect(move || log.borrow_mut().push(format!("{name}={}", f())));
}

/// Critério 1: nada roda dentro do `set`; `run_effects` roda tudo, de forma síncrona.
#[test]
fn efeitos_rodam_so_no_frame() {
    let scope = Scope::root();
    let log = Log::default();
    let a = scope.run(|| {
        let a = signal(0);
        logged(&log, "e", move || a.get().to_string());
        a
    });
    assert!(take(&log).is_empty(), "o efeito rodou antes do frame");

    run_effects();
    assert_eq!(take(&log), ["e=0"]);

    a.set(1);
    assert!(take(&log).is_empty(), "o efeito rodou dentro do set");
    run_effects();
    assert_eq!(take(&log), ["e=1"]);

    // Várias mudanças no mesmo frame viram uma execução só.
    a.set(2);
    a.set(3);
    a.update(|v| *v += 1);
    run_effects();
    assert_eq!(take(&log), ["e=4"]);

    assert_eq!(run_effects(), 0, "frame sem mudanças executou tarefas");
    scope.clear();
}

/// Critério 1: efeitos criados dentro de efeitos rodam no mesmo frame.
#[test]
fn efeitos_aninhados_rodam_no_mesmo_frame() {
    let scope = Scope::root();
    let log = Log::default();
    scope.run(|| {
        let a = signal(0);
        let inner = Rc::clone(&log);
        logged(&log, "fora", move || {
            let v = a.get();
            logged(&inner, "dentro", move || v.to_string());
            v.to_string()
        });
    });
    run_effects();
    assert_eq!(take(&log), ["fora=0", "dentro=0"]);
    scope.clear();
}

/// Dependências em diamante, com memos, e uma sequência fixa de mudanças.
fn scene() -> Vec<String> {
    let scope = Scope::root();
    let log = Log::default();
    let (n, text) = scope.run(|| {
        let n = signal(1);
        let text = signal(String::from("a"));
        let double = memo(move || n.get() * 2);
        let even = memo(move || double.get() % 4 == 0);
        logged(&log, "c", move || text.get());
        logged(&log, "a", move || n.get().to_string());
        logged(&log, "d", move || double.get().to_string());
        logged(&log, "b", move || format!("{}{}", text.get(), n.get()));
        logged(&log, "e", move || even.get().to_string());
        (n, text)
    });
    let frame = || {
        run_effects();
        log.borrow_mut().push("|".into());
    };
    frame();
    n.set(2);
    frame();
    text.set("b".into());
    n.set(3);
    frame();
    n.set(4);
    text.set("c".into());
    frame();
    scope.clear();
    take(&log)
}

/// Critério 2: a mesma sequência de mudanças produz sempre a mesma ordem. A ordem segue a
/// propagação (signal alterado primeiro, depois os inscritos), não a ordem de criação.
#[test]
fn ordem_deterministica() {
    let first = scene();
    assert_eq!(
        first.join(" "),
        "c=a a=1 d=2 b=a1 e=false | a=2 d=4 e=true b=a2 | c=b b=b3 a=3 d=6 e=false | \
         b=c4 a=4 d=8 e=true c=c |"
    );
    for _ in 0..100 {
        assert_eq!(scene(), first);
    }
}

/// Critério 2: cada teste do `TestApp` roda na sua thread; a ordem não pode depender dela.
#[test]
fn ordem_igual_entre_threads() {
    let here = scene();
    let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(scene)).collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), here);
    }
}

/// Base do repaint seletivo: memo com o mesmo resultado não notifica os dependentes.
#[test]
fn memo_igual_nao_propaga() {
    let scope = Scope::root();
    let log = Log::default();
    let n = scope.run(|| {
        let n = signal(2);
        let even = memo(move || n.get() % 2 == 0);
        logged(&log, "n", move || n.get().to_string());
        logged(&log, "par", move || even.get().to_string());
        n
    });
    run_effects();
    take(&log);

    n.set(4);
    run_effects();
    assert_eq!(take(&log), ["n=4"]);

    n.set(5);
    run_effects();
    assert_eq!(take(&log), ["n=5", "par=false"]);

    // `set` não compara: escrever o mesmo valor notifica quem lê o signal direto.
    n.set(5);
    run_effects();
    assert_eq!(take(&log), ["n=5"]);
    scope.clear();
}

/// Critério 3: o widget sai da árvore e leva os efeitos e as tarefas dele junto.
#[test]
fn widget_removido_descarta_efeitos() {
    let root = Scope::root();
    let log = Log::default();
    let tasks_before = live_tasks();
    let cleaned = Arc::new(Mutex::new(false));
    let widget = root.child();

    let a = root.run(|| signal(0));
    widget.run(|| {
        logged(&log, "w", move || a.get().to_string());
        let cleaned = Arc::clone(&cleaned);
        on_cleanup(move || *cleaned.lock().unwrap() = true);
    });
    root.run(|| logged(&log, "raiz", move || a.get().to_string()));
    run_effects();
    assert_eq!(take(&log), ["w=0", "raiz=0"]);
    assert_eq!(live_tasks(), tasks_before + 2);

    widget.clear();
    assert!(*cleaned.lock().unwrap(), "on_cleanup não rodou");

    a.set(1);
    run_effects();
    assert_eq!(take(&log), ["raiz=1"], "o efeito do widget continuou vivo");
    assert_eq!(live_tasks(), tasks_before + 1, "a tarefa do widget vazou");

    root.clear();
    run_effects();
    assert_eq!(live_tasks(), tasks_before);
}

/// Critério 3: o widget sai da árvore no meio do frame, com o efeito dele já na fila.
#[test]
fn widget_removido_durante_o_frame_nao_roda() {
    let root = Scope::root();
    let log = Log::default();
    let widget = root.child();

    let a = root.run(|| {
        let a = signal(0);
        // Criado antes, então roda antes e remove o filho quando `a` vira 1.
        let widget = widget.clone();
        logged(&log, "pai", move || {
            let v = a.get();
            if v == 1 {
                widget.clear();
            }
            v.to_string()
        });
        a
    });
    widget.run(|| logged(&log, "filho", move || a.get().to_string()));
    run_effects();
    assert_eq!(take(&log), ["pai=0", "filho=0"]);

    a.set(1);
    run_effects();
    assert_eq!(take(&log), ["pai=1"], "o filho rodou depois de descartado");
    root.clear();
}

/// Critério 3: remontar o widget (como faz o `Show::when`) não acumula efeitos antigos.
#[test]
fn remontar_nao_acumula() {
    let root = Scope::root();
    let log = Log::default();
    let widget = root.child();
    let a = root.run(|| signal(0));

    for _ in 0..50 {
        widget.clear();
        widget.run(|| logged(&log, "w", move || a.get().to_string()));
        run_effects();
    }
    take(&log);

    a.set(1);
    run_effects();
    assert_eq!(take(&log), ["w=1"]);
    root.clear();
}

/// Soltar o último handle de um escopo filho também o descarta.
#[test]
fn soltar_o_escopo_descarta() {
    let root = Scope::root();
    let log = Log::default();
    let a = root.run(|| signal(0));
    let widget = root.child();
    widget.run(|| logged(&log, "w", move || a.get().to_string()));
    run_effects();
    take(&log);

    drop(widget);
    a.set(1);
    run_effects();
    assert!(take(&log).is_empty(), "o efeito sobreviveu ao escopo");
    root.clear();
}

/// Soltar uma tarefa terminada pode rodar código (ex.: `on_cleanup`) que cria outra tarefa.
#[test]
fn tarefa_cujo_drop_cria_tarefa() {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// Fica pronta no primeiro `poll`, mas só solta o campo quando a caixa é descartada (um
    /// `async` soltaria as variáveis ainda dentro do `poll`).
    struct ReadyHolding(SpawnOnDrop);
    impl Future for ReadyHolding {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    struct SpawnOnDrop;
    impl Drop for SpawnOnDrop {
        fn drop(&mut self) {
            any_spawner::Executor::spawn_local(async {});
        }
    }

    init();
    let before = live_tasks();
    any_spawner::Executor::spawn_local(ReadyHolding(SpawnOnDrop));
    // Roda a tarefa; soltá-la cria outra, que também roda no mesmo frame.
    assert_eq!(run_effects(), 2);
    assert_eq!(live_tasks(), before);
}

#[test]
fn init_pode_ser_chamada_varias_vezes() {
    init();
    init();
    assert_eq!(run_effects(), 0);
}
