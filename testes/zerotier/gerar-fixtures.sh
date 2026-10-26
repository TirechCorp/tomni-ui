#!/usr/bin/env bash
# Gera as fixtures de identidade do crate `tomni-ui-zerotier` com o `zerotier-idtool` de verdade:
# identidades (com e sem a parte secreta) e uma mensagem assinada por cada uma. Os testes
# `tests/identidade.rs` conferem que o nosso código aceita o que o ZeroTier gerou e que o
# ZeroTier aceita o que o nosso código assina.
#
# Uso: gerar-fixtures.sh [quantas]   (padrão: 3)
set -euo pipefail

qtd="${1:-3}"
raiz="$(cd "$(dirname "$0")/../.." && pwd)"
destino="$raiz/crates/zerotier/tests/fixtures/identidades"
mkdir -p "$destino"
rm -f "$destino"/*

command -v zerotier-idtool >/dev/null || { echo "faltou o zerotier-idtool no PATH" >&2; exit 1; }

printf 'Mensagem de teste do Tomni UI para assinar.\n' > "$destino/mensagem.txt"
for i in $(seq 1 "$qtd"); do
    zerotier-idtool generate "$destino/id$i.secret" "$destino/id$i.public" >/dev/null
    zerotier-idtool sign "$destino/id$i.secret" "$destino/mensagem.txt" > "$destino/id$i.assinatura"
    zerotier-idtool validate "$destino/id$i.secret" >/dev/null
    echo "id$i: $(cut -d: -f1 "$destino/id$i.public")"
done
# O `zerotier-idtool` sem argumentos imprime a versão na primeira linha e sai com erro.
{ zerotier-idtool 2>&1 || true; } | head -1 > "$destino/VERSAO"
