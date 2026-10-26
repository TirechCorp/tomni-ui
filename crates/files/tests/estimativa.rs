//! A estimativa de tempo da fila (M40): sequências sintéticas no `Estimator`, sem thread e sem
//! relógio. Cada teste diz no nome o que confere.

use std::time::Duration;

use tomni_ui_files::TransferKind;
use tomni_ui_files::estimate::{Estimator, Eta, Route, Work};

const MIB: u64 = 1 << 20;

fn rota(n: u64) -> Route {
    Route {
        kind: TransferKind::Upload,
        from: 0,
        to: n,
    }
}

/// Alimenta a rota com `segundos` de vazão constante, em trechos de `trecho` segundos.
fn aprender(est: &mut Estimator, rota: Route, bytes_por_s: f64, segundos: f64, trecho: f64) {
    let trechos = (segundos / trecho).round() as u32;
    for _ in 0..trechos {
        est.bytes(rota, (bytes_por_s * trecho).round() as u64, trecho);
    }
}

/// Um item já na fase dos bytes: só faltam `bytes`.
fn so_bytes(rota: Route, bytes: u64) -> Work {
    Work {
        route: rota,
        files: 0,
        bytes,
    }
}

fn s(duracao: Duration) -> f64 {
    duracao.as_secs_f64()
}

fn razao(eta: &Eta) -> f64 {
    s(eta.high) / s(eta.low)
}

#[test]
fn vazao_estavel_da_um_tempo_justo_e_uma_faixa_estreita() {
    let r = rota(1);
    let mut est = Estimator::new();
    for _ in 0..100 {
        est.bytes(r, MIB, 0.1);
    }
    let eta = est.estimate(&[so_bytes(r, 100 * MIB)], 1).unwrap();
    assert!(
        (s(eta.likely) - 10.0).abs() < 0.2,
        "provável {:?}",
        eta.likely
    );
    assert!(
        razao(&eta) > 1.2 && razao(&eta) < 1.4,
        "razão {}",
        razao(&eta)
    );
}

#[test]
fn vazao_oscilante_alarga_a_faixa_e_acerta_a_media() {
    let r = rota(1);
    let mut est = Estimator::new();
    // 30 s em trechos de 0,5 s, alternando 2 e 20 MiB/s.
    for i in 0..60 {
        let vazao = if i % 2 == 0 { 2.0 } else { 20.0 };
        est.bytes(r, (vazao * MIB as f64 * 0.5) as u64, 0.5);
    }
    let curta = est.estimate(&[so_bytes(r, 110 * MIB)], 1).unwrap();
    assert!(
        (s(curta.likely) - 10.0).abs() < 1.5,
        "provável {:?}",
        curta.likely
    );
    assert!(razao(&curta) > 3.0, "razão {}", razao(&curta));

    // Com mais por mover, as oscilações se compensam e a faixa estreita.
    let longa = est.estimate(&[so_bytes(r, 1100 * MIB)], 1).unwrap();
    assert!(
        razao(&longa) < razao(&curta),
        "{} contra {}",
        razao(&longa),
        razao(&curta)
    );
}

#[test]
fn muitos_arquivos_pequenos_pesam_pelo_custo_fixo() {
    let r = rota(1);
    let mut est = Estimator::new();
    // O plano fala em 200 arquivos, mas 200 trechos de 0,002 s medem só 0,4 s, abaixo dos 2 s
    // de vazão medida que a estimativa exige: 1100 trechos passam dos 2 s.
    for _ in 0..1100 {
        est.bytes(r, 4096, 0.002);
        est.file_done(r, 0.05);
    }
    let faltam: Vec<Work> = (0..1000)
        .map(|_| Work {
            route: r,
            files: 1,
            bytes: 4096,
        })
        .collect();
    let um = est.estimate(&faltam, 1).unwrap();
    assert!((s(um.likely) - 52.0).abs() < 5.2, "P = 1: {:?}", um.likely);
    let quatro = est.estimate(&faltam, 4).unwrap();
    assert!(
        (s(quatro.likely) - 13.0).abs() < 1.3,
        "P = 4: {:?}",
        quatro.likely
    );
    // Só pelos bytes, 1000 × 4 KiB a 2 MB/s dariam uns 2 s.
    let so_pelos_bytes = 1000.0 * 4096.0 / (4096.0 / 0.002);
    assert!(s(um.likely) > 20.0 * so_pelos_bytes);
}

#[test]
fn uma_pausa_nao_entra_na_media() {
    let r = rota(1);
    let trabalho = [so_bytes(r, 100 * MIB)];
    let mut est = Estimator::new();
    let mut agora = Duration::ZERO;
    let mut feitos = 0_u64;
    let ticar = |est: &mut Estimator, ativo: bool, agora: &mut Duration, feitos: &mut u64| {
        *agora += Duration::from_secs(1);
        if ativo {
            *feitos += 10 * MIB;
        }
        est.tick(*agora, *feitos, ativo)
    };

    for _ in 0..30 {
        est.bytes(r, MIB, 0.1);
    }
    for _ in 0..10 {
        ticar(&mut est, true, &mut agora, &mut feitos);
    }
    let antes = est.estimate(&trabalho, 1).unwrap();
    let velocidade_antes = est.queue_speed();
    assert!((velocidade_antes - 10.0 * MIB as f64).abs() < 0.1 * MIB as f64);

    // 60 s de pausa: o relógio anda, nenhuma amostra entra.
    for _ in 0..60 {
        ticar(&mut est, false, &mut agora, &mut feitos);
    }
    assert_eq!(est.queue_speed(), velocidade_antes);

    // De volta: o primeiro tick ativo só põe a base e os seguintes repetem o ritmo de antes.
    for _ in 0..3 {
        ticar(&mut est, true, &mut agora, &mut feitos);
        assert!(
            (est.queue_speed() - velocidade_antes).abs() < 0.1 * MIB as f64,
            "velocidade {}",
            est.queue_speed()
        );
    }
    for _ in 0..30 {
        est.bytes(r, MIB, 0.1);
    }
    let depois = est.estimate(&trabalho, 1).unwrap();
    assert!((s(depois.likely) - s(antes.likely)).abs() < 0.05);
    assert!((razao(&depois) - razao(&antes)).abs() < 0.01);
}

#[test]
fn falha_nao_vira_custo() {
    let r = rota(1);
    let mut est = Estimator::new();
    aprender(&mut est, r, 10.0 * MIB as f64, 3.0, 0.1);
    for _ in 0..5 {
        est.file_done(r, 0.05);
    }
    // Um arquivo que falhou não chama `file_done`: o custo continua 0,05 s por arquivo.
    let faltam: Vec<Work> = (0..10)
        .map(|_| Work {
            route: r,
            files: 1,
            bytes: 0,
        })
        .collect();
    let eta = est.estimate(&faltam, 1).unwrap();
    assert!((s(eta.likely) - 0.5).abs() < 0.01, "{:?}", eta.likely);
}

#[test]
fn as_rotas_nao_se_misturam() {
    let (rapida, lenta) = (rota(1), rota(2));
    let mut est = Estimator::new();
    aprender(&mut est, rapida, 100.0 * MIB as f64, 3.0, 0.1);
    aprender(&mut est, lenta, MIB as f64, 3.0, 0.1);

    let so_a_lenta = est.estimate(&[so_bytes(lenta, 10 * MIB)], 1).unwrap();
    assert!((s(so_a_lenta.likely) - 10.0).abs() < 0.2);

    let as_duas = est
        .estimate(&[so_bytes(rapida, 100 * MIB), so_bytes(lenta, 10 * MIB)], 1)
        .unwrap();
    // 1 s da rápida + 10 s da lenta, com um trabalhador só.
    assert!(
        (s(as_duas.likely) - 11.0).abs() < 0.3,
        "{:?}",
        as_duas.likely
    );
}

#[test]
fn sem_dados_suficientes_nao_estima() {
    let r = rota(1);
    let trabalho = [so_bytes(r, 100 * MIB)];

    let mut pouco = Estimator::new();
    aprender(&mut pouco, r, 10.0 * MIB as f64, 1.5, 0.1);
    assert!(pouco.estimate(&trabalho, 1).is_none());
    let mut bastante = Estimator::new();
    aprender(&mut bastante, r, 10.0 * MIB as f64, 2.5, 0.1);
    assert!(bastante.estimate(&trabalho, 1).is_some());

    let dez_arquivos: Vec<Work> = (0..10)
        .map(|_| Work {
            route: r,
            files: 1,
            bytes: 0,
        })
        .collect();
    let mut custos = Estimator::new();
    custos.file_done(r, 0.05);
    custos.file_done(r, 0.05);
    assert!(custos.estimate(&dez_arquivos, 1).is_none());
    custos.file_done(r, 0.05);
    assert!(custos.estimate(&dez_arquivos, 1).is_some());

    // Sem nada por fazer, também não há o que dizer.
    assert!(custos.estimate(&[], 1).is_none());
}

#[test]
fn o_ultimo_arquivo_grande_nao_se_divide() {
    let r = rota(1);
    let mut est = Estimator::new();
    aprender(&mut est, r, 10.0 * MIB as f64, 3.0, 0.1);
    let grande = so_bytes(r, 1024 * MIB);
    let sozinho = est.estimate(&[grande], 1).unwrap();

    let mut itens = vec![grande];
    itens.extend((0..10).map(|_| so_bytes(r, 1024)));
    let com_pequenos = est.estimate(&itens, 4).unwrap();
    assert!(com_pequenos.likely >= sozinho.likely);
}

#[test]
fn a_faixa_e_ordenada_e_positiva() {
    let mut est = Estimator::new();
    let (a, b) = (rota(1), rota(2));
    aprender(&mut est, a, 10.0 * MIB as f64, 5.0, 0.1);
    for i in 0..40 {
        let vazao = if i % 2 == 0 { 1.0 } else { 30.0 };
        est.bytes(b, (vazao * MIB as f64 * 0.25) as u64, 0.25);
    }
    for i in 0..20 {
        est.file_done(a, 0.02 + 0.01 * f64::from(i % 3));
        est.file_done(b, 0.1);
    }
    for paralelas in [1, 2, 8] {
        for arquivos in [0, 1, 7, 300] {
            for mib in [0, 1, 50, 5000] {
                let trabalho: Vec<Work> = [a, b]
                    .into_iter()
                    .flat_map(|r| {
                        (0..3).map(move |_| Work {
                            route: r,
                            files: arquivos,
                            bytes: mib * MIB,
                        })
                    })
                    .collect();
                let Some(eta) = est.estimate(&trabalho, paralelas) else {
                    assert!(
                        arquivos == 0 && mib == 0,
                        "sem estimativa para {arquivos}/{mib}"
                    );
                    continue;
                };
                assert!(
                    Duration::ZERO < eta.low && eta.low <= eta.likely && eta.likely <= eta.high,
                    "P={paralelas} arquivos={arquivos} MiB={mib}: {eta:?}"
                );
            }
        }
    }
}
