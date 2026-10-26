# Planos

**Português** · [English](../en/ROADMAP.md)

O que está planejado, sem data e sem ordem de prioridade. Um item aqui não é promessa: se
quiser ajudar em algum, abra uma issue antes para combinar a abordagem
([CONTRIBUTING](CONTRIBUTING.md)).

## Mais idiomas e escritas

A parte da lib no plano de idiomas do
[Tirech Omni](https://github.com/TirechCorp/tirech-omni/blob/main/docs/pt-BR/ROADMAP.md):

- **Fontes para outras escritas.** A Selawik cobre só o alfabeto latino; chinês, japonês e
  coreano já existem pela feature `cjk` (cerca de 4,5 MB). Árabe, hebraico, devanágari,
  tailandês e outras pedem fontes Noto embutidas (por feature, como a `cjk`) ou o uso das
  fontes do sistema.
- **Escrita da direita para a esquerda.** Espelhar o layout dos widgets (`Row`, `NavigationView`,
  ícones com direção, foco por Tab) para árabe e hebraico; a ordem do texto (bidi) o parley já
  resolve.
- **Busca e quebra de linha** em línguas sem espaço entre as palavras.
- **Textos embutidos (`Strings`) por idioma.** Hoje o padrão é só em português e o app troca
  cada campo; a ideia é trazer conjuntos prontos (`Strings::en()`, por exemplo).
- **Lint:** conferir texto cortado e glifo faltando em cada idioma do app.
