//! A fila de transferências sobre o `Local` e o `MemFs`: pastas, paralelas, conflitos,
//! retomar, pausar, cancelar e apagar.

mod comum;

use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use tomni_ui_files::{
    Conflict, Endpoint, ManualClock, MemFs, QueueState, TransferInfo, TransferKind, TransferState,
    TransferSummary, Transfers,
};

use comum::{Pasta, dados};

const ESPERA: Duration = Duration::from_secs(20);

fn remoto(mem: &MemFs) -> Endpoint {
    Endpoint::Remote(mem.opener())
}

fn item(fila: &Transfers, id: u64) -> TransferInfo {
    fila.items()
        .get_untracked()
        .into_iter()
        .find(|i| i.id == id)
        .expect("o item está na lista")
}

fn esperar_ate(fila: &Transfers, id: u64, pronto: impl Fn(&TransferInfo) -> bool) -> TransferInfo {
    let limite = Instant::now() + ESPERA;
    loop {
        let atual = item(fila, id);
        if pronto(&atual) {
            return atual;
        }
        assert!(Instant::now() < limite, "tempo esgotado: {atual:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn terminar(fila: &Transfers) {
    assert!(fila.wait_idle(ESPERA), "a fila não esvaziou");
}

fn resumo(fila: &Transfers) -> TransferSummary {
    fila.summary().get_untracked()
}

/// Espera, no relógio de verdade, até `pronto` valer para a lista da fila.
fn esperar_lista(fila: &Transfers, pronto: impl Fn(&[TransferInfo]) -> bool) -> Vec<TransferInfo> {
    let limite = Instant::now() + ESPERA;
    loop {
        let itens = fila.items().get_untracked();
        if pronto(&itens) {
            return itens;
        }
        assert!(Instant::now() < limite, "tempo esgotado: {itens:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn envia_um_arquivo_com_a_data_da_origem() {
    let pasta = Pasta::nova("envio");
    let origem = pasta.escrever("relatório final.txt", &dados(300_000));
    let data = UNIX_EPOCH + Duration::from_secs(1_400_000_000);
    std::fs::File::options()
        .write(true)
        .open(&origem)
        .unwrap()
        .set_modified(data)
        .unwrap();
    let mem = MemFs::new();
    mem.dir("/srv");

    let fila = Transfers::new(2);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("relatório final.txt")),
        (remoto(&mem), "/srv/relatório final.txt".to_owned()),
        Conflict::Overwrite,
    );
    terminar(&fila);

    let info = item(&fila, id);
    assert_eq!(info.state, TransferState::Done);
    assert_eq!(info.kind, TransferKind::Upload);
    assert_eq!(info.size, Some(300_000));
    assert_eq!(info.done, 300_000);
    assert_eq!(
        mem.contents("/srv/relatório final.txt"),
        Some(dados(300_000))
    );
    let remoto = mem.connect().stat_for_test("/srv/relatório final.txt");
    assert_eq!(remoto.modified, Some(data));
}

trait StatParaTeste {
    fn stat_for_test(&mut self, path: &str) -> tomni_ui_files::Entry;
}

impl StatParaTeste for MemFs {
    fn stat_for_test(&mut self, path: &str) -> tomni_ui_files::Entry {
        use tomni_ui_files::Fs;
        self.stat(path).unwrap()
    }
}

#[test]
fn baixa_uma_pasta_inteira_sem_seguir_link_para_pasta() {
    let mem = MemFs::new();
    mem.put("/site/index.html", b"<html>")
        .put("/site/img/logo grande.png", dados(5000))
        .put("/site/img/fundo/azul.png", b"azul")
        .dir("/site/vazia")
        .link("/site/ciclo", "/site")
        .link("/site/atalho.html", "index.html");
    let pasta = Pasta::nova("baixa");

    let fila = Transfers::new(2);
    fila.add(
        (remoto(&mem), "/site".to_owned()),
        (Endpoint::Local, pasta.texto("copia")),
        Conflict::Overwrite,
    );
    terminar(&fila);

    let em = |p: &str| pasta.caminho().join("copia").join(p);
    assert_eq!(std::fs::read(em("index.html")).unwrap(), b"<html>");
    assert_eq!(
        std::fs::read(em("img/logo grande.png")).unwrap(),
        dados(5000)
    );
    assert_eq!(std::fs::read(em("img/fundo/azul.png")).unwrap(), b"azul");
    assert!(em("vazia").is_dir());
    assert!(!em("ciclo").exists(), "link para pasta fica de fora");
    assert_eq!(
        std::fs::read(em("atalho.html")).unwrap(),
        b"<html>",
        "link para arquivo vira o arquivo"
    );
    assert!(
        fila.items()
            .get_untracked()
            .iter()
            .all(|i| i.state == TransferState::Done || i.state == TransferState::Skipped),
        "{:?}",
        fila.items().get_untracked()
    );
}

#[test]
fn as_paralelas_andam_juntas_e_o_limite_vale() {
    for (paralelas, esperado) in [(1, 2), (3, 6)] {
        let mem = MemFs::new();
        mem.dir("/destino");
        mem.set_chunk(8 * 1024);
        mem.set_delay(Duration::from_millis(2));
        let pasta = Pasta::nova("paralelas");
        for n in 0..6 {
            pasta.escrever(&format!("f{n}.bin"), &dados(64 * 1024));
        }
        let fila = Transfers::new(paralelas);
        for n in 0..6 {
            fila.add(
                (Endpoint::Local, pasta.texto(&format!("f{n}.bin"))),
                (remoto(&mem), format!("/destino/f{n}.bin")),
                Conflict::Overwrite,
            );
        }
        terminar(&fila);
        // Cada transferência mantém um arquivo remoto aberto (a gravação).
        let abertos = mem.max_open_files();
        assert!(
            abertos <= paralelas,
            "com {paralelas} paralelas houve {abertos} arquivos abertos"
        );
        if paralelas > 1 {
            assert!(abertos > 1, "as paralelas não andaram juntas");
        }
        let _ = esperado;
        for n in 0..6 {
            assert_eq!(
                mem.contents(&format!("/destino/f{n}.bin")),
                Some(dados(64 * 1024))
            );
        }
    }
}

#[test]
fn conflito_pular_renomear_e_sobrescrever() {
    let pasta = Pasta::nova("conflito");
    pasta.escrever("a.txt", b"novo");
    let mem = MemFs::new();
    mem.put("/srv/a.txt", b"velho");
    let fila = Transfers::new(1);
    let ir = |conflito| {
        fila.add(
            (Endpoint::Local, pasta.texto("a.txt")),
            (remoto(&mem), "/srv/a.txt".to_owned()),
            conflito,
        )
    };

    let pular = ir(Conflict::Skip);
    terminar(&fila);
    assert_eq!(item(&fila, pular).state, TransferState::Skipped);
    assert_eq!(mem.contents("/srv/a.txt"), Some(b"velho".to_vec()));

    let renomear = ir(Conflict::Rename);
    terminar(&fila);
    assert_eq!(item(&fila, renomear).state, TransferState::Done);
    assert_eq!(item(&fila, renomear).to, "/srv/a (1).txt");
    assert_eq!(mem.contents("/srv/a (1).txt"), Some(b"novo".to_vec()));
    let outra = ir(Conflict::Rename);
    terminar(&fila);
    assert_eq!(item(&fila, outra).to, "/srv/a (2).txt");

    let trocar = ir(Conflict::Overwrite);
    terminar(&fila);
    assert_eq!(item(&fila, trocar).state, TransferState::Done);
    assert_eq!(mem.contents("/srv/a.txt"), Some(b"novo".to_vec()));
}

#[test]
fn retomar_continua_do_tamanho_do_destino_e_so_se_for_menor() {
    let pasta = Pasta::nova("retomar");
    let tudo = dados(100_000);
    pasta.escrever("grande.bin", &tudo);
    let mem = MemFs::new();
    mem.put("/srv/grande.bin", &tudo[..40_000]);
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("grande.bin")),
        (remoto(&mem), "/srv/grande.bin".to_owned()),
        Conflict::Resume,
    );
    terminar(&fila);
    assert_eq!(item(&fila, id).state, TransferState::Done);
    assert_eq!(mem.contents("/srv/grande.bin"), Some(tudo.clone()));

    // Destino maior que a origem: sobrescreve do começo.
    let mut maior = tudo.clone();
    maior.extend_from_slice(&[9; 500]);
    mem.put("/srv/grande.bin", &maior);
    fila.add(
        (Endpoint::Local, pasta.texto("grande.bin")),
        (remoto(&mem), "/srv/grande.bin".to_owned()),
        Conflict::Resume,
    );
    terminar(&fila);
    assert_eq!(mem.contents("/srv/grande.bin"), Some(tudo));
}

#[test]
fn perguntar_espera_a_resposta_e_o_aplicar_a_todos_vale_para_o_resto() {
    let pasta = Pasta::nova("perguntar");
    for nome in ["a.txt", "b.txt", "c.txt"] {
        pasta.escrever(nome, b"novo");
    }
    let mem = MemFs::new();
    for nome in ["a.txt", "b.txt", "c.txt"] {
        mem.put(&format!("/srv/{nome}"), b"velho!");
    }
    let fila = Transfers::new(2);
    let ids: Vec<u64> = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .map(|nome| {
            fila.add(
                (Endpoint::Local, pasta.texto(nome)),
                (remoto(&mem), format!("/srv/{nome}")),
                Conflict::Ask,
            )
        })
        .collect();

    let limite = Instant::now() + ESPERA;
    let pergunta = loop {
        if let Some(p) = fila.asking().get_untracked() {
            break p;
        }
        assert!(Instant::now() < limite, "nenhuma pergunta");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(pergunta.source.size, 4);
    assert_eq!(pergunta.target.size, 6);
    assert!(pergunta.path.starts_with("/srv/"));
    assert_eq!(item(&fila, pergunta.id).state, TransferState::Asking);
    assert_eq!(mem.contents(&pergunta.path), Some(b"velho!".to_vec()));

    fila.answer(pergunta.id, Conflict::Skip, true);
    terminar(&fila);
    assert!(fila.asking().get_untracked().is_none());
    for id in ids {
        assert_eq!(item(&fila, id).state, TransferState::Skipped);
    }
    assert_eq!(mem.contents("/srv/c.txt"), Some(b"velho!".to_vec()));
}

#[test]
fn cancelar_deixa_o_parcial_e_retomar_termina() {
    let pasta = Pasta::nova("cancelar");
    let tudo = dados(2 * 1024 * 1024);
    pasta.escrever("g.bin", &tudo);
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.set_chunk(16 * 1024);
    mem.set_delay(Duration::from_millis(5));
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("g.bin")),
        (remoto(&mem), "/srv/g.bin".to_owned()),
        Conflict::Overwrite,
    );
    esperar_ate(&fila, id, |i| i.done > 0 && i.speed > 0.0);
    fila.cancel(id);
    let info = esperar_ate(&fila, id, |i| i.state.is_finished());
    assert_eq!(info.state, TransferState::Cancelled);
    let parcial = mem.contents("/srv/g.bin").expect("o parcial fica");
    assert!(!parcial.is_empty() && parcial.len() < tudo.len());
    assert_eq!(parcial, tudo[..parcial.len()]);

    mem.set_delay(Duration::ZERO);
    let de_novo = fila.add(
        (Endpoint::Local, pasta.texto("g.bin")),
        (remoto(&mem), "/srv/g.bin".to_owned()),
        Conflict::Resume,
    );
    terminar(&fila);
    assert_eq!(item(&fila, de_novo).state, TransferState::Done);
    assert_eq!(mem.contents("/srv/g.bin"), Some(tudo));
}

#[test]
fn cancelar_no_meio_de_um_buffer_nao_espera_ele_acabar() {
    // O arquivo cabe num buffer só, e o servidor grava 10 bytes a cada 25 ms: inteiro, levaria
    // 10 s.
    let pasta = Pasta::nova("cancelar-buffer");
    pasta.escrever("p.bin", &dados(4000));
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.set_chunk(10);
    mem.set_delay(Duration::from_millis(25));
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("p.bin")),
        (remoto(&mem), "/srv/p.bin".to_owned()),
        Conflict::Overwrite,
    );
    esperar_ate(&fila, id, |i| i.state == TransferState::Running);
    let inicio = Instant::now();
    fila.cancel(id);
    let info = esperar_ate(&fila, id, |i| i.state.is_finished());
    assert_eq!(info.state, TransferState::Cancelled);
    assert!(
        inicio.elapsed() < Duration::from_secs(2),
        "o cancelar esperou o buffer: {:?}",
        inicio.elapsed()
    );
    let parcial = mem.contents("/srv/p.bin").unwrap_or_default();
    assert!(parcial.len() < 4000, "não gravou tudo");
}

#[test]
fn pausar_segura_o_que_anda_e_o_que_espera() {
    let pasta = Pasta::nova("pausar");
    pasta.escrever("p.bin", &dados(1024 * 1024));
    pasta.escrever("q.bin", &dados(1000));
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.set_chunk(16 * 1024);
    mem.set_delay(Duration::from_millis(5));
    let fila = Transfers::new(1);
    let p = fila.add(
        (Endpoint::Local, pasta.texto("p.bin")),
        (remoto(&mem), "/srv/p.bin".to_owned()),
        Conflict::Overwrite,
    );
    let q = fila.add(
        (Endpoint::Local, pasta.texto("q.bin")),
        (remoto(&mem), "/srv/q.bin".to_owned()),
        Conflict::Overwrite,
    );
    esperar_ate(&fila, p, |i| i.done > 0);
    fila.pause_all();
    assert!(fila.is_paused());
    esperar_ate(&fila, p, |i| i.state == TransferState::Paused);
    assert_eq!(item(&fila, q).state, TransferState::Paused);
    let parado = mem.contents("/srv/p.bin").unwrap().len();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        mem.contents("/srv/p.bin").unwrap().len(),
        parado,
        "andou parado"
    );

    mem.set_delay(Duration::ZERO);
    fila.resume_all();
    terminar(&fila);
    assert_eq!(item(&fila, p).state, TransferState::Done);
    assert_eq!(item(&fila, q).state, TransferState::Done);
    assert_eq!(mem.contents("/srv/p.bin"), Some(dados(1024 * 1024)));
}

#[test]
fn falha_mostra_o_motivo_e_tentar_de_novo_funciona() {
    let pasta = Pasta::nova("falha");
    let mem = MemFs::new();
    mem.dir("/srv");
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("ainda-nao-existe.txt")),
        (remoto(&mem), "/srv/x.txt".to_owned()),
        Conflict::Overwrite,
    );
    terminar(&fila);
    match item(&fila, id).state {
        TransferState::Failed(motivo) => assert!(motivo.contains("não encontrado"), "{motivo}"),
        outro => panic!("devia falhar: {outro:?}"),
    }

    pasta.escrever("ainda-nao-existe.txt", b"agora sim");
    fila.retry(id);
    terminar(&fila);
    assert_eq!(item(&fila, id).state, TransferState::Done);
    assert_eq!(mem.contents("/srv/x.txt"), Some(b"agora sim".to_vec()));

    fila.clear_finished();
    assert!(fila.items().get_untracked().is_empty());
}

#[test]
fn apaga_uma_pasta_por_dentro_pela_fila() {
    let mem = MemFs::new();
    mem.put("/lixo/a.txt", b"a")
        .put("/lixo/fundo/b.txt", b"b")
        .put("/lixo/fundo/mais/c.txt", b"c")
        .put("/fica/d.txt", b"d");
    let fila = Transfers::new(2);
    let id = fila.delete(remoto(&mem), "/lixo");
    terminar(&fila);
    let info = item(&fila, id);
    assert_eq!(info.state, TransferState::Done);
    assert_eq!(info.kind, TransferKind::Delete);
    assert_eq!(info.done, 6, "5 itens por dentro mais a própria pasta");
    assert!(!mem.exists("/lixo"));
    assert_eq!(mem.contents("/fica/d.txt"), Some(b"d".to_vec()));
}

#[test]
fn o_progresso_sobe_sem_voltar_e_a_velocidade_aparece() {
    let pasta = Pasta::nova("progresso");
    pasta.escrever("v.bin", &dados(1024 * 1024));
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.set_chunk(16 * 1024);
    mem.set_delay(Duration::from_millis(4));
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("v.bin")),
        (remoto(&mem), "/srv/v.bin".to_owned()),
        Conflict::Overwrite,
    );
    let mut ultimo = 0;
    let mut viu_velocidade = false;
    let limite = Instant::now() + ESPERA;
    loop {
        let info = item(&fila, id);
        assert!(info.done >= ultimo, "o progresso voltou");
        ultimo = info.done;
        viu_velocidade |= info.speed > 0.0;
        if info.state.is_finished() {
            assert_eq!(info.state, TransferState::Done);
            break;
        }
        assert!(Instant::now() < limite);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(ultimo, 1024 * 1024);
    assert!(viu_velocidade);
}

#[test]
fn copia_entre_dois_servidores() {
    let a = MemFs::new();
    a.put("/x/dados.csv", dados(10_000));
    let b = MemFs::new();
    b.dir("/y");
    let fila = Transfers::new(1);
    let id = fila.add(
        (remoto(&a), "/x/dados.csv".to_owned()),
        (remoto(&b), "/y/dados.csv".to_owned()),
        Conflict::Overwrite,
    );
    terminar(&fila);
    assert_eq!(item(&fila, id).kind, TransferKind::Copy);
    assert_eq!(b.contents("/y/dados.csv"), Some(dados(10_000)));
}

#[test]
fn o_tamanho_vem_da_listagem_da_pasta() {
    let mem = MemFs::new();
    for nome in ["a", "b", "c"] {
        mem.put(&format!("/p/{nome}.bin"), dados(200_000));
    }
    mem.set_chunk(16 * 1024);
    mem.set_delay(Duration::from_millis(100));
    let pasta = Pasta::nova("tamanho-da-listagem");
    let fila = Transfers::new(1);
    fila.add(
        (remoto(&mem), "/p".to_owned()),
        (Endpoint::Local, pasta.texto("copia")),
        Conflict::Overwrite,
    );

    let itens = esperar_lista(&fila, |itens| {
        itens
            .iter()
            .any(|i| i.state == TransferState::Running && !i.is_dir && i.done > 0)
    });
    let pasta_item = itens.iter().find(|i| i.is_dir).expect("o item da pasta");
    assert_eq!(pasta_item.state, TransferState::Done);
    let esperando: Vec<_> = itens
        .iter()
        .filter(|i| !i.is_dir && i.state == TransferState::Queued)
        .collect();
    assert_eq!(esperando.len(), 2, "{itens:?}");
    assert!(
        esperando.iter().all(|i| i.size == Some(200_000)),
        "{esperando:?}"
    );

    mem.set_delay(Duration::ZERO);
    terminar(&fila);
}

#[test]
fn as_subpastas_sao_listadas_antes_dos_arquivos() {
    let mem = MemFs::new();
    for n in 0..5 {
        mem.put(&format!("/a/arquivo{n}.bin"), dados(100_000));
    }
    mem.put("/a/sub/x", b"x");
    mem.set_chunk(16 * 1024);
    mem.set_delay(Duration::from_millis(50));
    let pasta = Pasta::nova("subpastas-primeiro");
    let fila = Transfers::new(1);
    fila.add(
        (remoto(&mem), "/a".to_owned()),
        (Endpoint::Local, pasta.texto("copia")),
        Conflict::Overwrite,
    );

    let itens = esperar_lista(&fila, |itens| {
        itens.iter().any(|i| {
            i.state == TransferState::Running && !i.is_dir && i.from.starts_with("/a/arquivo")
        })
    });
    assert!(
        itens.iter().any(|i| i.from == "/a/sub/x"),
        "o item de /a/sub/x devia existir: {itens:?}"
    );
    mem.set_delay(Duration::ZERO);
    terminar(&fila);
    assert_eq!(
        std::fs::read(pasta.caminho().join("copia/sub/x")).unwrap(),
        b"x"
    );
}

#[test]
fn o_resumo_conta_e_depois_soma() {
    let mem = MemFs::new();
    mem.put("/p/a.bin", dados(3000))
        .put("/p/b.bin", dados(5000))
        .put("/p/fundo/c.bin", dados(7000))
        .put("/p/fundo/mais/d.bin", dados(11));
    let pasta = Pasta::nova("resumo-soma");
    let fila = Transfers::new(2);

    fila.pause_all();
    fila.add(
        (remoto(&mem), "/p".to_owned()),
        (Endpoint::Local, pasta.texto("copia")),
        Conflict::Overwrite,
    );
    fila.refresh_summary();
    let antes = resumo(&fila);
    assert!(antes.counting, "{antes:?}");
    assert_eq!(antes.state, QueueState::Paused);
    assert_eq!(antes.speed, 0.0);
    assert_eq!(antes.eta, None);

    fila.resume_all();
    terminar(&fila);
    let depois = resumo(&fila);
    assert!(!depois.counting, "{depois:?}");
    assert_eq!(depois.state, QueueState::Idle);
    assert_eq!(depois.files_total, 4);
    assert_eq!(depois.files_done, 4);
    assert_eq!(depois.failed, 0);
    assert_eq!(depois.bytes_total, 3000 + 5000 + 7000 + 11);
    assert_eq!(depois.bytes_done, depois.bytes_total);
    assert_eq!(depois.eta, None);
    assert_eq!(depois.stalled, None);
}

#[test]
fn uma_leva_nova_nao_conta_a_anterior() {
    let pasta = Pasta::nova("leva-nova");
    pasta.escrever("um.txt", b"um");
    pasta.escrever("dois.txt", b"dois");
    let mem = MemFs::new();
    mem.dir("/srv");
    let fila = Transfers::new(1);

    fila.add(
        (Endpoint::Local, pasta.texto("um.txt")),
        (remoto(&mem), "/srv/um.txt".to_owned()),
        Conflict::Overwrite,
    );
    terminar(&fila);
    assert_eq!(resumo(&fila).files_total, 1);

    fila.add(
        (Endpoint::Local, pasta.texto("dois.txt")),
        (remoto(&mem), "/srv/dois.txt".to_owned()),
        Conflict::Overwrite,
    );
    terminar(&fila);
    let segunda = resumo(&fila);
    assert_eq!(
        (segunda.files_total, segunda.files_done),
        (1, 1),
        "{segunda:?}"
    );
    assert_eq!(
        fila.items().get_untracked().len(),
        2,
        "as linhas antigas continuam"
    );

    // Um item que falha e volta pelo `retry` com a fila parada também começa uma leva nova.
    fila.add(
        (Endpoint::Local, pasta.texto("depois.txt")),
        (remoto(&mem), "/srv/depois.txt".to_owned()),
        Conflict::Overwrite,
    );
    let falho = fila.items().get_untracked().last().map(|i| i.id).unwrap();
    terminar(&fila);
    assert_eq!(resumo(&fila).failed, 1);
    pasta.escrever("depois.txt", b"depois");
    fila.retry(falho);
    terminar(&fila);
    let leva = resumo(&fila);
    assert_eq!(
        (leva.files_total, leva.files_done, leva.failed),
        (1, 1, 0),
        "{leva:?}"
    );
}

#[test]
fn falha_fica_fora_do_total() {
    let pasta = Pasta::nova("falha-no-total");
    pasta.escrever("bom.txt", b"bom");
    pasta.escrever("ruim.txt", b"ruim");
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.deny("/srv/ruim.txt");
    let fila = Transfers::new(1);
    for nome in ["bom.txt", "ruim.txt"] {
        fila.add(
            (Endpoint::Local, pasta.texto(nome)),
            (remoto(&mem), format!("/srv/{nome}")),
            Conflict::Overwrite,
        );
    }
    terminar(&fila);
    let leva = resumo(&fila);
    assert_eq!(leva.failed, 1, "{leva:?}");
    assert_eq!((leva.files_total, leva.files_done), (1, 1), "{leva:?}");
    assert_eq!(leva.state, QueueState::Idle);
}

#[test]
fn a_estimativa_aparece_e_a_linha_tem_restante() {
    let pasta = Pasta::nova("estimativa");
    for nome in ["a", "b", "c"] {
        pasta.escrever(&format!("{nome}.bin"), &dados(2 * 1024 * 1024));
    }
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.set_chunk(64 * 1024);
    mem.set_delay(Duration::from_millis(50));
    let fila = Transfers::new(1);
    // Uma ponta só: a rota (e a vazão medida) é a mesma para os três arquivos.
    let destino = remoto(&mem);
    for nome in ["a", "b", "c"] {
        fila.add(
            (Endpoint::Local, pasta.texto(&format!("{nome}.bin"))),
            (destino.clone(), format!("/srv/{nome}.bin")),
            Conflict::Overwrite,
        );
    }

    let limite = Instant::now() + Duration::from_secs(30);
    let (eta, em_andamento) = loop {
        let leva = resumo(&fila);
        let linha = fila
            .items()
            .get_untracked()
            .into_iter()
            .find(|i| i.state == TransferState::Running && i.remaining.is_some());
        if let (Some(eta), Some(linha)) = (leva.eta, linha) {
            break (eta, linha);
        }
        assert!(
            Instant::now() < limite,
            "a estimativa não apareceu: {leva:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(eta.low <= eta.likely && eta.likely <= eta.high, "{eta:?}");
    assert!(eta.likely < Duration::from_secs(10), "{eta:?}");
    let restante = em_andamento.remaining.unwrap();
    assert!(restante.low <= restante.likely && restante.likely <= restante.high);

    mem.set_delay(Duration::ZERO);
    terminar(&fila);
    assert_eq!(resumo(&fila).eta, None);
    assert!(
        fila.items()
            .get_untracked()
            .iter()
            .all(|i| i.remaining.is_none()),
        "{:?}",
        fila.items().get_untracked()
    );
}

#[test]
fn o_tamanho_dado_ao_enfileirar_tira_a_leva_do_contando() {
    let pasta = Pasta::nova("tamanho-dado");
    pasta.escrever("a.bin", &dados(1024 * 1024));
    pasta.escrever("b.bin", &dados(1024 * 1024));
    let mem = MemFs::new();
    mem.dir("/srv");
    let fila = Transfers::new(1);
    fila.pause_all();
    let destino = remoto(&mem);
    fila.add(
        (Endpoint::Local, pasta.texto("a.bin")),
        (destino.clone(), "/srv/a.bin".to_owned()),
        Conflict::Overwrite,
    );
    fila.refresh_summary();
    assert!(resumo(&fila).counting, "sem tamanho, a leva está contando");

    fila.add_sized(
        (Endpoint::Local, pasta.texto("b.bin")),
        (destino, "/srv/b.bin".to_owned()),
        Conflict::Overwrite,
        Some(1024 * 1024),
    );
    let itens = fila.items().get_untracked();
    assert_eq!(itens[0].size, None);
    assert_eq!(itens[1].size, Some(1024 * 1024));
    fila.refresh_summary();
    // O `a.bin` ainda não tem tamanho: a leva só sai do "contando" quando a fila o lê.
    assert!(resumo(&fila).counting);

    fila.resume_all();
    terminar(&fila);
    let fim = resumo(&fila);
    assert!(!fim.counting);
    assert_eq!(fim.bytes_total, 2 * 1024 * 1024);
}

#[test]
fn a_velocidade_nao_despenca_depois_da_pausa() {
    let pasta = Pasta::nova("pausa-velocidade");
    pasta.escrever("lento.bin", &dados(1024 * 1024));
    let mem = MemFs::new();
    mem.dir("/srv");
    mem.set_chunk(16 * 1024);
    mem.set_delay(Duration::from_millis(20));
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("lento.bin")),
        (remoto(&mem), "/srv/lento.bin".to_owned()),
        Conflict::Overwrite,
    );

    esperar_ate(&fila, id, |i| i.done > 256 * 1024 && i.speed > 0.0);
    fila.pause_all();
    let parado = esperar_ate(&fila, id, |i| i.state == TransferState::Paused);
    let antes = parado.speed;
    // Mais que os 500 ms do plano: com a base de tempo antiga, uma pausa curta quase não se
    // vê na média, e esta tem de pegar o defeito.
    std::thread::sleep(Duration::from_millis(1500));
    fila.resume_all();
    let retomado = esperar_ate(&fila, id, |i| i.state == TransferState::Running);
    let depois = esperar_ate(&fila, id, |i| {
        i.done >= retomado.done + 3 * 64 * 1024 || i.state == TransferState::Done
    });
    assert!(
        depois.speed > 0.7 * antes,
        "a velocidade caiu de {antes:.0} para {:.0}",
        depois.speed
    );
    mem.set_delay(Duration::ZERO);
    terminar(&fila);
}

#[test]
fn o_relogio_manual_comanda_o_resumo() {
    let pasta = Pasta::nova("relogio-manual");
    let mem = MemFs::new();
    mem.put("/srv/preso.bin", dados(100_000));
    // Cada leitura fica presa: o item chega à fase dos bytes e não anda mais.
    mem.set_chunk(1024);
    mem.set_delay(Duration::from_secs(30));
    let relogio = Arc::new(ManualClock::new());
    let fila = Transfers::with_clock(1, relogio.clone());
    let id = fila.add(
        (remoto(&mem), "/srv/preso.bin".to_owned()),
        (Endpoint::Local, pasta.texto("preso.bin")),
        Conflict::Overwrite,
    );
    esperar_ate(&fila, id, |i| {
        i.state == TransferState::Running && i.size.is_some()
    });

    fila.refresh_summary();
    assert_eq!(resumo(&fila).at, Duration::ZERO);
    assert_eq!(resumo(&fila).stalled, None);

    relogio.advance(Duration::from_secs(2));
    fila.refresh_summary();
    assert_eq!(resumo(&fila).at, Duration::from_secs(2));
    assert_eq!(resumo(&fila).stalled, None, "2 s ainda não é parada");

    // Passados os 5 s sem um byte, a leva está parada. O item pode ainda estar chegando à fase
    // dos bytes, então tenta até aparecer.
    relogio.advance(Duration::from_secs(4));
    let limite = Instant::now() + ESPERA;
    let parada = loop {
        fila.refresh_summary();
        let leva = resumo(&fila);
        if leva.stalled.is_some() {
            break leva;
        }
        assert!(Instant::now() < limite, "não ficou parada: {leva:?}");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(parada.at, Duration::from_secs(6));
    assert!(
        parada.stalled.unwrap() >= Duration::from_secs(5),
        "{parada:?}"
    );
    assert_eq!(parada.eta, None);
    assert_eq!(parada.state, QueueState::Running);
}
