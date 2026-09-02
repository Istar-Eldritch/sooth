# tree-sitter-sooth

A Tree-sitter grammar for Sooth, for editor syntax highlighting only — it is
not a semantic parser and doesn't try to be one. `src/lexer.rs` already
emits almost everything as a generic `Word` token and leaves sigil/case
conventions (`^Type`, `&!x`, `Foo>bar`, `mod::word`, capitalised type names,
`if`/`branch`, clause-head patterns like `| Cons`) to the real parser and
checker, which carry a symbol table this grammar doesn't have. Some of those
(clause heads with no matching close-pipe) are genuinely undecidable from
tokens alone. So `grammar.js` mirrors the same split: only the unambiguous
structure — top-level `: ... ;` / `type: ... ;` / `extern:` / `trait:` / `impl:` /
`import:` / `export:` / `static:` forms, and the always-paired `(...)`/`[...]`/
`~[...]` delimiters — gets a grammar rule. Everything else is a flat `word`
token that `queries/highlights.scm` classifies by shape (regex/exact-set
predicates over the token text).

## Rebuilding

```sh
tree-sitter generate      # grammar.js -> src/parser.c
tree-sitter build -o sooth.so
```

`sooth.so` is gitignored; rebuild it after any `grammar.js` change.

## Neovim wiring

This repo doesn't install anything into your Neovim config automatically.
The Neovim config at `~/code/dotfiles/home/.config/nvim/init.lua` loads this
checkout's built `sooth.so` and `queries/highlights.scm` directly with core
`vim.treesitter`; it also maps `.sth` to the `sooth` filetype and starts the
parser. nvim-treesitter does not manage this local grammar.

Rebuild `sooth.so` after changing the grammar, then restart Neovim.

If `#match?` predicates are ever added to `highlights.scm`: don't. Neovim
compiles `#match?` patterns as Vim "very magic" (`\v`) regex, where a bare
`&` is the branch-AND operator — `"^&"` silently matches every string, not
just ones starting with `&`. Use `#lua-match?` (plain Lua patterns) or
`#any-of?` (exact sets) instead.
