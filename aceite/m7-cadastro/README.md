# Aceite do M7: cadastro feito por uma sessão nova

O `src/main.rs` está como a sessão nova do Claude Code (`claude -p`, Opus 5.5) o entregou no
teste final do M7, só formatado com o `cargo fmt` (a sessão não
o rodou, e o CI confere a formatação do workspace). O `Cargo.toml` mudou: as dependências
passaram a vir do workspace. O crate é membro do workspace para que o `cargo test` e o lint do
CI continuem conferindo que um app escrito só a partir do guia funciona com a lib atual.

## Como foi o teste

- Montagem igual à do M6: projeto vazio fora do repositório, com `cargo init`, `fluent-rs` e
  `fluent-rs-test` por caminho e `fluent-rs init`.
- A sessão tinha o MCP do `fluent-rs` e não podia ler o código da lib. Terminou sem
  intervenção, em 12 turnos (uns 96 s).
- Na primeira compilação, a sessão escreveu `app.click("termos").click("criar")`, que não
  compilava: só o `type_into` devolvia o app. Ela separou os cliques em duas linhas. Depois do
  M7, todas as ações do `TestApp` encadeiam.

## O pedido

> Crie neste projeto Rust uma tela de cadastro com o Fluent-RS.
>
> - No topo, o título "Criar conta" como texto (não como botão).
> - Campos de nome, e-mail e senha.
> - Uma caixa de seleção "Li e aceito os termos". O botão "Criar conta", em destaque, só fica
>   habilitado com ela marcada.
> - Uma chave liga/desliga "Receber novidades por e-mail", começando desligada.
> - Ao clicar em "Criar conta" com o e-mail ana@exemplo.com, aparece uma barra de erro com a
>   mensagem "Este e-mail já está cadastrado". Com qualquer outro e-mail, aparece uma barra de
>   sucesso dizendo que a conta foi criada. Antes do clique, nenhuma das duas barras aparece.
>
> Use como documentação só o que está neste projeto; não consulte o código-fonte do Fluent-RS.
> Escreva os testes e só termine com tudo pronto: testes passando e o lint sem erros.

## Conferir

```sh
cargo test -p m7-cadastro
cargo run -p fluent-rs-cli -- lint -p m7-cadastro
cargo run -p m7-cadastro          # abre a janela
```
