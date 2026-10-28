//! `AreaChart`, `Sparkline`, `LevelMeter` e o calor das células do `DataGrid`, nos dois temas:
//! a página do exemplo `desempenho` e o medidor em cada nível, com hover e com foco.

mod comum;

use comum::theme_name;
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
