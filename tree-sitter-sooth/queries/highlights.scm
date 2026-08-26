; Sooth highlighting. The grammar only gives us flat `word` tokens outside
; effects/quotations (see grammar.js), so most classification here is by
; pattern-matching over the token text — the same sigil/case conventions the
; compiler itself resolves with a symbol table (src/parser.rs, src/check.rs)
; rather than a fixed keyword set. Patterns are ordered general -> specific;
; a later pattern's capture wins over an earlier one on the same node.
;
; Uses #lua-match? (Lua patterns), not #match? (Vim "very magic" regex,
; where a bare `&` is the branch-AND operator — silently matches everything
; when used as a literal, which is exactly the sigil this grammar needs).

; --- literals ---
(int) @number
(float) @number.float
(string) @string
(comment) @comment

; --- structural keywords / punctuation ---
":" @keyword
"type:" @keyword
"extern:" @keyword
"trait:" @keyword
"impl:" @keyword
"import:" @keyword
"export:" @keyword
"static:" @keyword
";" @punctuation.delimiter
"|" @punctuation.special
["(" ")"] @punctuation.bracket
["[" "]"] @punctuation.bracket
(tilde_lbracket) @punctuation.bracket

; --- definition names (uses the grammar's field annotations) ---
(word_definition name: (word) @function)
(type_definition name: (word) @type)
(extern_definition name: (word) @function)
(trait_definition name: (word) @type)
(impl_definition trait: (word) @type)
(import_definition alias: (word) @module)
(static_definition name: (word) @variable)

; --- fallback classification of plain `word` tokens by shape ---

; capitalised bare word: a type or enum-variant name/constructor
((word) @type
 (#lua-match? @type "^%u"))

; poly type vars (`'T`, `'T:`) and row vars (`..s`)
((word) @type.parameter
 (#lua-match? @type.parameter "^'"))
((word) @type.parameter
 (#lua-match? @type.parameter "^%.%."))

; module-qualified word (`mod::word`)
((word) @module
 (#lua-match? @module "::"))

; field/place accessor containing a glued `>` (`Point>x`, `&Buf>data`, `^>`)
((word) @property
 (#lua-match? @property ">"))

; numeric/pointer conversion words (`>usize`, `>u8`, `>f64`)
((word) @function.builtin
 (#lua-match? @function.builtin "^>"))

; borrow sigils (`&x`, `&!x`, `&^`, `&!^`)
((word) @operator
 (#lua-match? @operator "^&"))

; owning-cell sigil, bare wrap verb or glued type (`^`, `^List`, `^Cons`)
((word) @constructor
 (#lua-match? @constructor "^%^"))

; effect arrow
((word) @punctuation.special
 (#lua-match? @punctuation.special "^%-%-$"))

; `global:` (a word, not a grammar literal) and the `impl:` separator `for`,
; the `inline` word modifier, and the `owning` quotation-type keyword.
((word) @keyword
 (#any-of? @keyword "global:" "for" "inline" "owning"))

; control flow / booleans / common stack-and-arith words: fixed sets, so an
; exact-match predicate (no pattern-syntax pitfalls) is both simplest and
; the most robust in the face of stray sigil chars.
; `else`/`end` are gone: slice 10c deleted the `if ... else ... end` keywords.
; `branch` is the primitive; `if`/`unless` are the `lib/core.sth` words over it.
((word) @keyword.conditional
 (#any-of? @keyword.conditional "if" "unless" "branch"))

((word) @boolean
 (#any-of? @boolean "true" "false"))

((word) @function.builtin
 (#any-of? @function.builtin
   ; stack shuffle
   "dup" "drop" "swap" "over" "rot" "nip" "tuck"
   ; arithmetic / bitwise
   "add" "sub" "mul" "div" "mod" "and" "or" "xor" "not" "shl" "shr"
   ; comparison primitives (32-bit flag)
   "ueq" "ult" "ugt" "ulte" "ugte" "une"
   ; surface comparisons (lib/cmp.sth words)
   "eq" "lt" "gt" "lte" "gte" "ne"
   ; control / discriminant
   "call" "tag" "branch"
   ; arrays / strings / slices
   "times" "fill" "len" "cstr" "slice" "subslice"
   ; print
   "."))
