# P7.S3r spike: does a delimiter-bearing synthesized word name survive the pipeline?

Throwaway spike in worktree `/root/code/ordfruma/sooth-slice3r-mangling`
(branch `spike/slice3r-mangling`, off `5338c06`). Tests brief decision 3's
untested claim (b): a word whose internal env name contains a lexer delimiter
(`cmp;Order;Point`) can be declared, resolved, mangled, lowered, emitted as
valid QBE, and linked.

## Method

One-line lexer hack (`src/lexer.rs`): a user word token `SPIKE` is rewritten to
the internal name `cmp;Order;Point`. Both the definition `: SPIKE ... ;` and
every call `SPIKE` then carry the delimiter name through the whole pipeline. A
throwaway `src/bin/dumpssa.rs` prints `driver::emit_ssa` output verbatim.

## Findings

### 1. resolve::mangle — survives, no exemption needed, none added

`mangle` is `format!("{name}__m{module}")` with a two-name exemption
(`main`/`drop`); it inspects no characters. The env name became
`cmp;Order;Point__m0` (module 0, not exempt). No exemption is hit and none is
required.

### 2. Emitted symbol is legal for QBE and the assembler/linker

`qbe_name` (`src/backend/qbe.rs`) escapes every char outside `[A-Za-z0-9_]` to
`.{hex}.` injectively (it already does this for hyphenated names like
`point-cmp` -> `.2d.`). `;` (0x3b) becomes `.3b.`. Emitted QBE (exact):

```
export function l $cmp.3b.Order.3b.Point__m0(l %v0) {
@start
	%v1 =l copy 1
	%v2 =l add %v0, %v1
	ret %v2
}
...
	%v1 =l call $cmp.3b.Order.3b.Point__m0(l %v0)
```

Definition and call site sanitize identically. `qbe examples/...` and `cc`
accepted it; the binary links and runs (exit 0). `nm` on the final binary:

```
0000000000001190 T cmp.3b.Order.3b.Point__m0
00000000000011a0 T sooth_main
```

### 3. Nothing downstream keys on the name's shape

The only name-shape splits in the tree are on `::` (module qualifier) and `__m`
(mangle suffix); a `;`-delimited name contains neither, so `demangle_word`,
poly dispatch, and the REPL alias logic are unaffected. No checker/IR assertion
validates that a word name is identifier-shaped or lexable; names are opaque
strings. (Not exercised: the REPL `dlopen` path specifically. `qbe_name` is
applied identically there, so the symbol should match, but I did not build a
REPL probe.)

### 4. Unforgeability confirmed

`;` is a hard lexer delimiter (`Token::Semicolon`), so a user source token can
never contain it. `: cmp;Order;Point ( ... )` lexes as `: cmp ;` and errors:

```
error: parse error: expected LParen, found Semicolon at line 2, col 6
```

So `;` is both unforgeable (hard delimiter) and pipeline-safe (escaped by
`qbe_name`). It is the cheapest working shape; any of `; ( ) | [ ]` would do
equally. `$`/`.` are NOT delimiters and remain forgeable (matches the brief).

### 5. DEFECT: the synthesized name leaks verbatim into user diagnostics

`demangle_word` strips only the trailing `__m{n}`; it has no notion of the
synthesized structure. Two diagnostics that name the word render the raw
internal spelling:

```
error: stack effect mismatch in `main` (line 4)
  `cmp;Order;Point` needs 1 values, but the stack holds 0
```

```
error: duplicate word `cmp;Order;Point` (line 3, col 3); first defined at ...
```

A user who mis-calls a trait member would see `cmp;Order;Point`, a name they
never wrote and cannot type. The spec must give diagnostics a way to render a
synthesized impl-member name back to something a user understands (e.g. `cmp`,
or "`cmp` for `Order` on `Point`"). This is a required spec ruling, not an
optional polish item: it is the one place the delimiter approach has a visible
cost.

## Verdict

Claim (b) holds: a delimiter-bearing internal name survives mangle, lowering,
emission, and linking with no new exemption. Decision 3 is sound on the
mechanism. The one gap it does not cover is diagnostic rendering (finding 5),
which the spec must address.
