//! `AreaChart`, `Sparkline`, `LevelMeter`, `HeatMap` e o calor das células do `DataGrid`, nos
//! dois temas: a página do exemplo `desempenho`, o medidor em cada nível, com hover e com foco,
//! e o mapa de calor de 64 núcleos com a dica de um ladrilho.

mod comum;

use comum::theme_name;
use tomni_ui::TOOLTIP_DELAY;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/desempenho.rs"]
mod desempenho;

fn medidores() -> impl View {
    Form::new("Medidores").child(
        Column::new().padding(8).gap(4).child(
            Row::new()
                .gap(4)
                .child(LevelMeter::new(0.0).id("zero"))
                .child(LevelMeter::new(0.35).id("pouco"))
                .child(LevelMeter::new(0.75).id("muito"))
                .child(LevelMeter::new(1.0).id("cheio").on_click(|| {})),
        ),
    )
}

#[test]
fn a_pagina_desempenho() {
    for tema in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(desempenho::painel).size(720, 560).theme(tema);
        app.click("amostrar")
            .click("amostrar")
            .click("amostrar")
            .click("amostrar");
        app.assert_snapshot(&format!("desempenho_{}", theme_name(tema)));
    }
}

#[test]
fn o_medidor_em_cada_nivel_com_hover_e_foco() {
    for tema in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(medidores).size(380, 44).theme(tema);
        app.assert_snapshot(&format!("level_meter_{}", theme_name(tema)));
        app.hover("cheio");
        app.assert_snapshot(&format!("level_meter_hover_{}", theme_name(tema)));
        app.tab();
        assert_eq!(app.focused().as_deref(), Some("cheio"));
        app.assert_snapshot(&format!("level_meter_foco_{}", theme_name(tema)));
    }
}

/// 64 núcleos com usos de 0 a 100% espalhados, num mapa de 3 vezes mais largo que alto.
fn mapa() -> impl View {
    let usos: Vec<f64> = (0..64).map(|i| (i * 37 % 101) as f64).collect();
    Form::new("Núcleos").child(
        Column::new().padding(8).child(
            HeatMap::new(usos)
                .id("nucleos")
                .cell_tooltip(|n, uso| format!("Núcleo {n}: {uso:.0}%"))
                .grow(1),
        ),
    )
}

#[test]
fn o_mapa_de_calor_com_a_dica_do_ladrilho() {
    for tema in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(mapa).size(380, 140).theme(tema);
        app.assert_lint();
        assert_eq!(app.values("nucleos").len(), 64);
        let area = app.rect("nucleos");
        assert_eq!(
            tomni_ui::heat_map_grid(64, area.size()),
            (13, 5),
            "{area:?}"
        );
        app.move_to(tomni_ui::heat_map_cell(64, area, 3).center());
        app.wait(TOOLTIP_DELAY + std::time::Duration::from_millis(10));
        assert!(app.is_open("nucleos-tooltip"));
        let arvore = app.tree();
        let dica = arvore.find("nucleos-tooltip").expect("a dica do mapa");
        let texto = dica.children.first().and_then(|t| t.text.clone());
        assert_eq!(texto.as_deref(), Some("Núcleo 3: 10%"));
        app.assert_snapshot(&format!("heat_map_{}", theme_name(tema)));
    }
}
