## O que muda

<!-- Uma ou duas frases. Se resolve uma issue, escreva "Resolve #123". -->

## Como foi conferido

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets --features tomni-ui-test/cjk,tomni-ui-test/gpu -- -D warnings`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- [ ] `cargo test --workspace` e `cargo test -p tomni-ui-test --features cjk`
- [ ] `cargo deny check licenses`
- [ ] o `tomni-ui lint` dos exemplos tocados
- [ ] snapshots novos ou mudados conferidos no `.snap.new.png` e no `.snap.diff.png`
- [ ] mudou a API pública: o `crates/cli/templates/AGENTS.md` e o `llms.txt` foram atualizados

## Capturas

<!-- Antes e depois, nos dois temas, se a mudança for visual. -->
