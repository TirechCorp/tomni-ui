#!/usr/bin/env python3
"""Gera o `Symbol` e os caminhos dos ícones a partir dos Fluent System Icons.

Uso: `python3 -I crates/theme/icons/gerar.py <cache>`, da raiz do repositório.

Baixa os SVGs de `lista.txt` (do commit fixado em `COMMIT`) para a pasta `<cache>`, que
deve ser nova ou uma usada antes por este script, e grava:

- `crates/core/src/symbol.rs`: o enum `Symbol`, com `name()` e `ALL`;
- `crates/theme/src/symbols.rs`: os caminhos de cada ícone nos tamanhos 12, 16, 20 e 24 que
  o Fluent desenhou;
- a lista de ícones do guia, entre os marcadores de `crates/cli/templates/AGENTS.md`.

Os SVGs são dados de fora: o script só lê os atributos `d` dos `<path>` e confere que têm
apenas os caracteres de um caminho SVG.
"""

import pathlib
import re
import sys
import urllib.error
import urllib.request

COMMIT = "a347af95478f989038cccbf085978865264c048a"
URL = "https://raw.githubusercontent.com/microsoft/fluentui-system-icons/{commit}/assets/{pasta}/SVG/ic_fluent_{nome}_{tam}_{variante}.svg"
TAMANHOS = (12, 16, 20, 24)
RAIZ = pathlib.Path(__file__).resolve().parents[3]
INICIO = "<!-- ícones: início (gerado por crates/theme/icons/gerar.py) -->"
FIM = "<!-- ícones: fim -->"

CAMINHO = re.compile(r"^[MmLlHhVvCcSsQqTtAaZz0-9eE.,\- ]+$")
PATH = re.compile(r"<path\b([^>]*)/?>")
D = re.compile(r'\bd="([^"]*)"')
PERMITIDOS = re.compile(r"^\s*(fill|d)=\"[^\"]*\"\s*$")


class Icone:
    def __init__(self, nome, variante, enum, categoria):
        self.nome = nome
        self.variante = variante
        self.enum = enum
        self.categoria = categoria
        self.caminhos = {}

    @property
    def fluent(self):
        return f"{self.nome}_{self.variante}"


def pascal(nome):
    return "".join(p[:1].upper() + p[1:] for p in nome.split("_"))


def ler_lista():
    icones = []
    categoria = None
    for linha in (RAIZ / "crates/theme/icons/lista.txt").read_text().splitlines():
        if linha.startswith("## "):
            categoria = linha[3:].strip()
            continue
        if not linha.strip() or linha.startswith("#"):
            continue
        partes = linha.split()
        nome, _, variante = partes[0].partition(":")
        variante = variante or "regular"
        enum = partes[1] if len(partes) > 1 else pascal(nome)
        if variante != "regular":
            enum += pascal(variante)
        icones.append(Icone(nome, variante, enum, categoria))
    enums = [i.enum for i in icones]
    repetidos = {e for e in enums if enums.count(e) > 1}
    if repetidos:
        sys.exit(f"nomes repetidos na lista: {sorted(repetidos)}")
    return icones


def pasta(nome):
    """`music_note_2` -> `Music Note 2`, como as pastas de `assets/`."""
    especiais = {"ltr": "LTR", "rtl": "RTL", "wifi": "WiFi"}
    return " ".join(especiais.get(p, p[:1].upper() + p[1:]) for p in nome.split("_"))


def baixar(cache, icone, tam):
    arquivo = cache / f"{icone.fluent}_{tam}.svg"
    ausente = cache / f"{icone.fluent}_{tam}.ausente"
    if arquivo.exists():
        return arquivo.read_text()
    if ausente.exists():
        return None
    url = URL.format(
        commit=COMMIT,
        pasta=urllib.request.quote(pasta(icone.nome)),
        nome=icone.nome,
        tam=tam,
        variante=icone.variante,
    )
    try:
        with urllib.request.urlopen(url, timeout=30) as resposta:
            texto = resposta.read(200_000).decode("utf-8")
    except urllib.error.HTTPError as erro:
        if erro.code == 404:
            ausente.write_text("")
            return None
        raise
    arquivo.write_text(texto)
    return texto


def caminho(svg, origem):
    if re.search(r"<(circle|rect|ellipse|line|polygon|polyline|g|use)\b|transform=", svg):
        sys.exit(f"{origem}: o SVG tem mais que <path> simples")
    partes = []
    for atributos in PATH.findall(svg):
        atributos = atributos.rstrip("/")
        for atributo in re.findall(r'\S+="[^"]*"', atributos):
            if not PERMITIDOS.match(atributo):
                sys.exit(f"{origem}: atributo inesperado {atributo[:40]!r}")
        d = D.search(atributos)
        if not d or not CAMINHO.match(d.group(1)):
            sys.exit(f"{origem}: caminho inválido")
        partes.append(d.group(1))
    if not partes:
        sys.exit(f"{origem}: sem <path>")
    return "".join(partes)


def gerar_symbol(icones):
    linhas = [
        "//! Os ícones do `Icon` e dos botões, dos Fluent System Icons (MIT, Microsoft).",
        "//!",
        "//! Gerado por `crates/theme/icons/gerar.py` a partir de `crates/theme/icons/lista.txt`;",
        "//! não edite à mão.",
        "",
        "/// Um ícone dos Fluent System Icons.",
        "///",
        "/// Os nomes seguem os do Fluent (`ic_fluent_search_20_regular` vira `Symbol::Search`), não",
        "/// os do enum `Symbol` do WinUI, que vêm da fonte Segoe Fluent Icons. As variantes",
        "/// `*Filled` são os desenhos cheios. A lista com as categorias está no guia dos apps.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]",
        "pub enum Symbol {",
    ]
    categoria = None
    for icone in icones:
        if icone.categoria != categoria:
            categoria = icone.categoria
            linhas.append(f"    // {categoria}")
        linhas.append(f"    /// `{icone.fluent}`.")
        linhas.append(f"    {icone.enum},")
    linhas += [
        "}",
        "",
        "impl Symbol {",
        "    /// Todos os ícones, na ordem do guia.",
        f"    pub const ALL: [Symbol; {len(icones)}] = [",
    ]
    linhas += [f"        Symbol::{i.enum}," for i in icones]
    linhas += [
        "    ];",
        "",
        "    /// O nome do ícone como se escreve no código (`\"Search\"`), o que a árvore mostra.",
        "    pub fn name(self) -> &'static str {",
        "        match self {",
    ]
    linhas += [f'            Symbol::{i.enum} => "{i.enum}",' for i in icones]
    linhas += [
        "        }",
        "    }",
        "",
        "    /// O nome no Fluent, sem o tamanho (`\"search_regular\"`).",
        "    pub fn fluent_name(self) -> &'static str {",
        "        match self {",
    ]
    linhas += [f'            Symbol::{i.enum} => "{i.fluent}",' for i in icones]
    linhas += ["        }", "    }", "}", ""]
    return "\n".join(linhas)


def gerar_caminhos(icones):
    linhas = [
        "//! Os caminhos dos ícones do `Symbol` (Fluent System Icons, MIT, Microsoft; a licença",
        "//! está em `icons/LICENSE`).",
        "//!",
        "//! Gerado por `icons/gerar.py` a partir de `icons/lista.txt`; não edite à mão.",
        "",
        "use tomni_ui_core::Symbol;",
        "",
        "/// Os desenhos de `symbol`: o lado do quadrado (12, 16, 20 ou 24) e o caminho SVG, do",
        "/// menor para o maior.",
        "#[rustfmt::skip]",
        "pub(crate) fn sizes(symbol: Symbol) -> &'static [(u8, &'static str)] {",
        "    match symbol {",
    ]
    for icone in icones:
        linhas.append(f"        Symbol::{icone.enum} => &[")
        for tam, d in sorted(icone.caminhos.items()):
            linhas.append(f'            ({tam}, "{d}"),')
        linhas.append("        ],")
    linhas += ["    }", "}", ""]
    return "\n".join(linhas)


def gerar_guia(icones):
    linhas = [INICIO, ""]
    categoria = None
    nomes = []
    for icone in icones + [None]:
        if icone is None or icone.categoria != categoria:
            if nomes:
                linhas.append(f"- **{categoria}**: " + ", ".join(f"`{n}`" for n in nomes) + ".")
            if icone is None:
                break
            categoria = icone.categoria
            nomes = []
        nomes.append(icone.enum)
    linhas += ["", FIM]
    return "\n".join(linhas)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    cache = pathlib.Path(sys.argv[1])
    cache.mkdir(parents=True, exist_ok=True)
    icones = ler_lista()
    for icone in icones:
        for tam in TAMANHOS:
            svg = baixar(cache, icone, tam)
            if svg is not None:
                icone.caminhos[tam] = caminho(svg, f"{icone.fluent}_{tam}")
        if not icone.caminhos:
            sys.exit(f"{icone.fluent}: nenhum tamanho entre {TAMANHOS}")
    (RAIZ / "crates/core/src/symbol.rs").write_text(gerar_symbol(icones))
    (RAIZ / "crates/theme/src/symbols.rs").write_text(gerar_caminhos(icones))
    guia = RAIZ / "crates/cli/templates/AGENTS.md"
    texto = guia.read_text()
    if INICIO in texto and FIM in texto:
        antes, resto = texto.split(INICIO, 1)
        _, depois = resto.split(FIM, 1)
        guia.write_text(antes + gerar_guia(icones) + depois)
    else:
        print("aviso: o guia não tem os marcadores da lista de ícones", file=sys.stderr)
    faltam = [f"{i.enum}: {sorted(set(TAMANHOS) - set(i.caminhos))}" for i in icones if len(i.caminhos) < 4]
    print(f"{len(icones)} ícones; sem algum tamanho: {len(faltam)}")
    for f in faltam:
        print("  ", f)


if __name__ == "__main__":
    main()
