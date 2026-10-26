# Aceite do M9: "Minha conta" feita por uma sessão nova

O `src/main.rs` está como a sessão nova do Claude Code (`claude -p`, Opus 5.5) o entregou no
teste final do M9, só formatado com o `cargo fmt` (a sessão não
o rodou, e o CI confere a formatação do workspace). O `Cargo.toml` mudou: as dependências
passaram a vir do workspace. O crate é membro do workspace para que o `cargo test` e o lint do
CI continuem conferindo que um app escrito só a partir do guia funciona com a lib atual.

## Como foi o teste

- Montagem igual à do M6 ao M8: projeto vazio fora do repositório, com `cargo new`,
  `fluent-rs` e `fluent-rs-test` por caminho e `fluent-rs init`.
- A sessão tinha o MCP do `fluent-rs` e não podia ler o código da lib. Terminou sem
  intervenção, em 11 turnos (uns 89 s), e compilou, passou nos testes e no lint na primeira
  tentativa.
- O pedido não dá o nome de nenhum widget ("lista suspensa", "menu", "caixa de confirmação",
  "barra de aviso"): a sessão escolheu `ComboBox`, `MenuFlyout`, `ContentDialog` e `InfoBar`
  pelo guia.
- Os testes abrem a lista, o menu e o dialog e chamam `assert_lint()` com eles abertos, como o
  guia pede. Ela também percorreu o menu, o dialog e o "Sair" no `ui_interact`.
- Por conta própria, acrescentou o título "Minha conta", rótulos acima dos campos e o X nas
  duas barras de aviso.

## O pedido

> Crie neste projeto Rust a tela "Minha conta" de um app, com o Fluent-RS.
>
> - Um campo com o nome da pessoa (começa com "Ana Souza") e a UF dela, escolhida numa lista
>   suspensa entre MG, RJ, SP, BA e RS (começa sem nenhuma escolhida).
> - Um botão "Mais opções" que abre um menu com "Sair" e "Excluir conta".
> - "Sair" mostra a mensagem "Você saiu da conta.".
> - "Excluir conta" abre uma caixa de confirmação "Excluir conta?", que explica que os dados
>   serão apagados, com os botões "Excluir" e "Cancelar". A conta só é excluída se a pessoa
>   confirmar em "Excluir"; depois disso, aparece uma barra de aviso de sucesso dizendo que a
>   conta foi excluída. Cancelar não muda nada.
>
> Use como documentação só o que está neste projeto; não consulte o código-fonte do Fluent-RS.
> Escreva os testes e só termine com tudo pronto: testes passando e o lint sem erros.

## Conferir

```sh
cargo test -p m9-conta
cargo run -p fluent-rs-cli -- lint -p m9-conta
cargo run -p m9-conta          # abre a janela
```
