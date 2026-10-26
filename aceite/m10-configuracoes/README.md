# Aceite do M10: "Configurações" feita por uma sessão nova

O `src/main.rs` está exatamente como a sessão nova do Claude Code (`claude -p`, Opus 5.5) o
entregou no teste final do M10; ela mesma rodou o `cargo fmt`.
O `Cargo.toml` mudou: as dependências passaram a vir do workspace. O crate é membro do
workspace para que o `cargo test` e o lint do CI continuem conferindo que um app escrito só a
partir do guia funciona com a lib atual.

## Como foi o teste

- Montagem igual à do M6 ao M9: projeto vazio fora do repositório, com `cargo new`,
  `fluent-rs` e `fluent-rs-test` por caminho e `fluent-rs init`.
- A sessão tinha o MCP do `fluent-rs` e não podia ler o código da lib. Terminou sem
  intervenção, em 17 turnos (uns 123 s).
- O pedido não dá o nome de nenhum widget ("duas abas", "sugere nomes enquanto a pessoa
  digita", "seção que abre e fecha", "barra de progresso que fica andando"): a sessão escolheu
  `TabView`, `AutoSuggestBox`, `Expander`, `Slider`, `RadioButtons` e `ProgressBar` pelo guia.
- Tropeços, todos corrigidos por ela a partir da mensagem do compilador ou do teste:
  - `Key::ArrowDown` no lugar de `Key::Down`;
  - `app.tree().to_string()`, mas o `tree()` devolve um `Node`, não texto;
  - `app.label("tema")` no grupo de rádio, que ainda não devolvia o cabeçalho.

  Depois do teste, o guia passou a listar as teclas e a dizer o que o `tree()` devolve, e o
  `label` do `RadioButtons` passou a devolver o cabeçalho.
- Os testes abrem a lista de cidades e a seção e chamam `assert_lint()` com elas abertas, nos
  dois temas, e conferem que ir e voltar entre as abas mantém a cidade, o volume e o tema.
  Ela também conferiu a lista aberta e o "Avançado" no `ui_interact`.

## O pedido

> Crie neste projeto Rust a tela "Configurações" de um app, com o Fluent-RS.
>
> - A tela tem duas abas, "Geral" e "Avançado".
> - Em "Geral", um campo "Cidade" que sugere nomes de cidades enquanto a pessoa digita (por
>   exemplo, Belo Horizonte, Belém, Brasília, Curitiba, Fortaleza, Porto Alegre, Recife, Rio
>   de Janeiro, Salvador e São Paulo), e uma barra de progresso de "Sincronizando" que fica
>   andando sem um fim definido.
> - Em "Avançado", uma seção que abre e fecha, chamada "Som e aparência", com o volume (de 0 a
>   100, começa em 50) e a escolha do tema entre "Claro", "Escuro" e "Do sistema" (começa em
>   "Do sistema").
> - O que a pessoa escreveu ou escolheu numa aba continua lá quando ela volta para essa aba.
>
> Use como documentação só o que está neste projeto; não consulte o código-fonte do Fluent-RS.
> Escreva os testes e só termine com tudo pronto: testes passando e o lint sem erros.

## Conferir

```sh
cargo test -p m10-configuracoes
cargo run -p fluent-rs-cli -- lint -p m10-configuracoes
cargo run -p m10-configuracoes          # abre a janela
```
