# Aceite do M6: login feito por uma sessão nova

O `src/main.rs` está como a sessão nova do Claude Code (`claude -p`, Opus 5.5) o entregou no
teste final do M6. Só o `Cargo.toml` mudou: as dependências
passaram a vir do workspace. O crate é membro do workspace para que o `cargo test` e o lint do
CI continuem conferindo que um app escrito só a partir do guia funciona com a lib atual.

## Como foi o teste

- Projeto vazio fora do repositório: `cargo init`, `fluent-rs` e `fluent-rs-test` por caminho
  e `fluent-rs init`, que gravou o `AGENTS.md` (o de `crates/cli/templates`), o `CLAUDE.md`, a
  skill e o `.mcp.json`.
- A sessão tinha o MCP do `fluent-rs` e não podia ler o código da lib. Ela terminou sem
  intervenção, em 13 turnos (uns 90 s).
- O guia da época dizia que o `.grow(n)` ocupava "o espaço livre". Por isso a sessão escreveu
  `.width(0).grow(1)` para os dois botões ficarem do mesmo tamanho. Depois do M6, `.grow(1)`
  sozinho já faz isso, e o `.width(0)` ficou sem efeito.

## O pedido

> Crie neste projeto Rust uma tela de login com o Fluent-RS.
>
> - Campos de e-mail e de senha.
> - Um botão "Entrar", em destaque, que só fica habilitado quando o e-mail tem "@" e a senha
>   tem pelo menos 8 caracteres.
> - Ao lado do "Entrar", um botão "Criar conta".
> - Ao clicar em "Entrar" com o e-mail ana@exemplo.com e a senha segredo123, o botão passa a
>   mostrar "Bem-vindo, Ana!". Com qualquer outra combinação, ele mostra "Senha incorreta" até
>   a pessoa mudar a senha.
>
> Use como documentação só o que está neste projeto; não consulte o código-fonte do Fluent-RS.
> Escreva os testes e só termine com tudo pronto: testes passando e o lint sem erros.

## Conferir

```sh
cargo test -p m6-login
cargo run -p fluent-rs-cli -- lint -p m6-login
cargo run -p m6-login          # abre a janela
```
