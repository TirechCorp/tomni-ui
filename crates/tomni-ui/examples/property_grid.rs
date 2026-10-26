//! `PropertyGrid`: a grade de propriedades de um objeto. `cargo run --example property_grid`.
//!
//! Cada propriedade é uma linha com o nome e o valor; a linha escolhida troca o valor pelo
//! editor do tipo dela (`.text`, `.password`, `.number`, `.choice`, `.check`, `.file` e `.folder`).
//! `.category` agrupa, `.default` põe em negrito o que difere do padrão, `.description` aparece
//! no painel de baixo e `.read_only` e `.disabled` travam a linha. Os valores são `Signal`s do
//! app, e a grade recebe closures: trocar de conexão na lista troca os signals sem recriar a
//! grade. O texto, a senha e o número só vão para o app ao confirmar (Enter, Tab ou perder o
//! foco; o Esc desfaz). A `.file` tem o botão "…", que abre o seletor de arquivo do sistema, e a
//! `.folder` tem o mesmo botão com o seletor de pasta. O
//! `.button` mostra o resumo de uma coleção ("Nenhum", "2 itens") com o "…" que a edita num
//! diálogo (aqui, só conta mais um item). `.hidden` esconde uma linha (ou, depois de uma
//! `.category`, a categoria inteira) enquanto ela não vale: "Endereço do proxy" só aparece com
//! "Usar proxy" marcado, e o valor digitado fica guardado quando some.
//! As linhas têm os ids `{id}-{nome}` e os editores `{id}-{nome}-valor`. No `TestApp`:
//! `app.properties("props")`, `app.click("props-Porta")` e `app.file_dialog_answer(Some(..))`.

use tomni_ui::prelude::*;

/// Os valores de uma conexão.
#[derive(Clone, Copy)]
struct Conexao {
    host: Signal<String>,
    porta: Signal<f64>,
    protocolo: Signal<String>,
    usuario: Signal<String>,
    senha: Signal<String>,
    chave: Signal<String>,
    pasta: Signal<String>,
    bipe: Signal<bool>,
    itens: Signal<u32>,
}

impl Conexao {
    fn nova(host: &str, porta: f64, protocolo: &str, usuario: &str) -> Self {
        Self {
            host: signal(host.to_owned()),
            porta: signal(porta),
            protocolo: signal(protocolo.to_owned()),
            usuario: signal(usuario.to_owned()),
            senha: signal(String::new()),
            chave: signal(String::new()),
            pasta: signal(String::new()),
            bipe: signal(true),
            itens: signal(0),
        }
    }
}

/// O resumo do `.button`: "Nenhum", "1 item" ou "N itens", sempre em dia com `itens`.
fn resumo(itens: impl Fn() -> Signal<u32> + 'static) -> impl Fn() -> Signal<String> {
    let texto = signal(String::new());
    effect(move || {
        texto.set(match itens().get() {
            0 => "Nenhum".to_owned(),
            1 => "1 item".to_owned(),
            n => format!("{n} itens"),
        });
    });
    move || texto
}

/// As conexões na lista à esquerda e, à direita, a grade da escolhida.
pub fn conexoes() -> impl View {
    let producao = Conexao::nova("web-01.exemplo.com", 22.0, "SSH", "deploy");
    let legado = Conexao::nova("10.0.0.7", 23.0, "Telnet", "admin");
    let console = Conexao::nova("/dev/ttyUSB0", 115_200.0, "Serial", "");
    let escolhida = signal("Produção".to_owned());
    let usar_proxy = signal(false);
    let endereco_proxy = signal(String::new());
    let atual = move || match escolhida.get().as_str() {
        "Legado" => legado,
        "Console" => console,
        _ => producao,
    };
    Form::new("Conexões").child(
        Row::new()
            .grow(1)
            .padding(12)
            .gap(12)
            .child(
                RadioButtons::bind(escolhida, ["Produção", "Legado", "Console"])
                    .id("conexoes")
                    .header("Conexões"),
            )
            .child(
                PropertyGrid::new()
                    .id("props")
                    .grow(1)
                    .category("Conexão")
                    .text("Host", move || atual().host)
                    .description("Nome ou endereço do servidor.")
                    .default("")
                    .number("Porta", move || atual().porta, 1.0..=65535.0)
                    .description("A porta TCP; no modo serial, a velocidade.")
                    .default(22)
                    .choice(
                        "Protocolo",
                        move || atual().protocolo,
                        ["SSH", "Telnet", "Serial"],
                    )
                    .description("Como o terminal fala com o servidor.")
                    .default("SSH")
                    .check("Usar proxy", usar_proxy)
                    .description("Liga o endereço do proxy logo abaixo.")
                    .text("Endereço do proxy", endereco_proxy)
                    .description("Só aparece com \"Usar proxy\" marcado; o texto não se perde.")
                    .hidden(move || !usar_proxy.get())
                    .category("Autenticação")
                    .text("Usuário", move || atual().usuario)
                    .description("A conta usada no login.")
                    .password("Senha", move || atual().senha)
                    .description("Fica só na memória; aparece mascarada.")
                    .file("Chave privada", move || atual().chave)
                    .description("O arquivo da chave, em vez da senha.")
                    .filter("Chaves", &["pem", "key"])
                    .folder("Pasta de trabalho", move || atual().pasta)
                    .description("Onde a sessão começa; o \"…\" escolhe uma pasta.")
                    .category("Terminal")
                    .check("Bipe", move || atual().bipe)
                    .description("Toca um som quando o servidor envia o sinal de alerta.")
                    .default(true)
                    .button("Itens", resumo(move || atual().itens), move || {
                        atual().itens.update(|n| *n += 1)
                    })
                    .description("Uma coleção editada à parte: o \"…\" adiciona um item."),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(conexoes).size(720, 560).run()
}
