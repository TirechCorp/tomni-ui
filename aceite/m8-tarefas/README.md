# Aceite do M8: lista de tarefas feita por uma sessão nova

O `src/main.rs` está como a sessão nova do Claude Code (`claude -p`, Opus 5.5) o entregou no
teste final do M8, só formatado com o `cargo fmt` (a sessão não
o rodou, e o CI confere a formatação do workspace). O `Cargo.toml` mudou: as dependências
passaram a vir do workspace. O crate é membro do workspace para que o `cargo test` e o lint do
CI continuem conferindo que um app escrito só a partir do guia funciona com a lib atual.

## Como foi o teste

- Montagem igual à do M6 e do M7: projeto vazio fora do repositório, com `cargo init`,
  `fluent-rs` e `fluent-rs-test` por caminho e `fluent-rs init`.
- A sessão tinha o MCP do `fluent-rs` e não podia ler o código da lib. Terminou sem
  intervenção, em 11 turnos (uns 105 s).
- Na primeira compilação, a sessão escreveu `app.rect("tarefa-2").y`, que não compilava: o
  `Rect` do `kurbo` tem `x0`, `y0`, `x1` e `y1`. O compilador sugeriu `y0`, e ela corrigiu.
  Depois do M8, o guia diz quais são os campos do `Rect`.
- Ela conferiu a lista com 15 tarefas no `ui_interact`, além do lint nos dois temas.

## O pedido

> Crie neste projeto Rust uma lista de tarefas com o Fluent-RS.
>
> - No topo, um campo para o texto da tarefa e o botão "Adicionar", que põe a tarefa no fim
>   da lista e limpa o campo. Com o campo vazio, o botão fica desabilitado.
> - Cada tarefa aparece com uma caixa de seleção com o texto dela (marcada quando feita) e um
>   botão para removê-la.
> - Um contador "N pendentes", com o número de tarefas não marcadas.
> - Com a lista vazia, aparece "Nenhuma tarefa" no lugar da lista.
> - A lista rola quando não cabe na tela. Teste com 50 tarefas: a última precisa dar para
>   alcançar e marcar.
>
> Use como documentação só o que está neste projeto; não consulte o código-fonte do Fluent-RS.
> Escreva os testes e só termine com tudo pronto: testes passando e o lint sem erros.

## Conferir

```sh
cargo test -p m8-tarefas
cargo run -p fluent-rs-cli -- lint -p m8-tarefas
cargo run -p m8-tarefas          # abre a janela
```
