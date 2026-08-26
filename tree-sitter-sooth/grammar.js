// Tree-sitter grammar for Sooth — syntax highlighting only.
//
// Sooth's own lexer (src/lexer.rs) emits almost everything as a generic
// `Word` token and leaves sigil/case conventions (`^Type`, `&!x`, `Foo>bar`,
// `mod::word`, capitalised type/variant names, `'T` poly vars, `if`/`branch`,
// `dup`/`drop`/...) to the parser and checker, which both carry a
// symbol table this grammar doesn't have. Some of those conventions (clause
// heads like `| Cons` with no matching close-pipe) are genuinely undecidable
// from tokens alone. So this grammar mirrors the same split: only the truly
// unambiguous structure — top-level `: ... ;` / `type: ... ;` / `trait:` /
// `impl:` / ... forms and real paired `(...)`/`[...]`/`~[...]` delimiters —
// gets a grammar rule; everything else is a flat `word` token, and
// highlights.scm classifies it by regex.

module.exports = grammar({
	name: "sooth",

	extras: ($) => [/\s/, $.comment],

	word: ($) => $.word,

	rules: {
		source_file: ($) => repeat($._toplevel),

		_toplevel: ($) =>
			choice(
				$.word_definition,
				$.type_definition,
				$.extern_definition,
				$.trait_definition,
				$.impl_definition,
				$.import_definition,
				$.export_definition,
				$.static_definition,
			),

		word_definition: ($) => seq(":", field("name", $.word), repeat($._atom), ";"),
		type_definition: ($) =>
			seq("type:", field("name", $.word), repeat($._atom), ";"),
		extern_definition: ($) =>
			seq("extern:", field("name", $.word), repeat($._atom), ";"),
		// `trait:` has no nested `;` — member signatures are `( ... )` groups,
		// each a single atom, so the simple `repeat($._atom)` body works.
		trait_definition: ($) =>
			seq("trait:", field("name", $.word), repeat($._atom), ";"),
		// `impl:` contains member bodies (`: word ... ;`), each ending in `;`,
		// so `;` must be an admissible atom inside the block. The greedy `repeat`
		// backtracks one step so the final `;` terminates the declaration.
		impl_definition: ($) =>
			seq("impl:", field("trait", $.word), repeat(choice($._atom, ";")), ";"),
		import_definition: ($) =>
			seq(
				"import:",
				field("alias", choice($.word, $.string)),
				repeat($._atom),
				";",
			),
		export_definition: ($) => seq("export:", repeat($._atom), ";"),
		static_definition: ($) =>
			seq("static:", field("name", $.word), repeat($._atom), ";"),

		// A stack effect `( ... )`, a quotation/array-type `[ ... ]`, and an
		// inline quotation `~[ ... ]` are the genuinely unambiguous, always-
		// paired delimiters in the language; everything they contain is just
		// more atoms. `~[` is a single token (matching src/lexer.rs's
		// `TildeLBracket`, Slice 10a R1) — `~` glued to `[` with zero whitespace
		// — so `~ [` (spaced) still lexes as `word("~")` + `"["` and is a parse
		// error in the real compiler, not a silently-accepted quotation.
		_atom: ($) =>
			choice(
				$.word,
				$.int,
				$.float,
				$.string,
				"|",
				$.paren_group,
				$.bracket_group,
				$.tilde_bracket_group,
			),
		paren_group: ($) => seq("(", repeat($._atom), ")"),
		// `;` is admissible inside `[ ... ]` for the `[ Type ; Count ]` array
		// constructor (Slice 6h D1), distinguished from a quotation `[ -- ]`
		// by the real parser's depth scan — not replicable here, so both are
		// accepted, which is harmless for highlighting.
		bracket_group: ($) => seq("[", repeat(choice($._atom, ";")), "]"),
		tilde_lbracket: (_) => token(prec(2, "~[")),
		tilde_bracket_group: ($) => seq($.tilde_lbracket, repeat($._atom), "]"),

		// Base word chars exclude `; ( ) | [ ] "` and whitespace, except that a
		// mid-word `|` glues into the token when immediately followed by `>`
		// (the `S|>fi` peek-word rule in lexer.rs) — but only after at least one
		// ordinary char, so a leading `|` always stays the standalone delimiter.
		word: (_) =>
			token(
				prec(1, seq(/[^\s;()|\x5b\]"]/, repeat(choice(/[^\s;()|\x5b\]"]/, "|>")))),
			),

		int: (_) => token(prec(2, /-?[0-9]+/)),
		float: (_) => token(prec(2, /-?[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?/)),

		string: (_) =>
			token(seq('"', repeat(choice(/[^"\\\n]/, seq("\\", /./))), '"')),

		// `\` starts a comment only when it stands alone as its own word (i.e.
		// is immediately followed by whitespace or EOF) — `\x` glued to more
		// text is just an ordinary (almost always erroneous) word to the real
		// lexer, not a comment.
		comment: (_) => token(prec(2, choice(seq("\\", /[ \t][^\n]*/), "\\"))),
	},
});
