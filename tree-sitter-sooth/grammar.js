// Tree-sitter grammar for Sooth — syntax highlighting only.
//
// Sooth's own lexer (src/lexer.rs) emits almost everything as a generic
// `Word` token and leaves sigil/case conventions (`^Type`, `&!x`, `Foo>bar`,
// `mod::word`, capitalised type/variant names, `'T` poly vars, `if`/`branch`,
// `dup`/`drop`/...) to the parser and checker, which both carry a
// symbol table this grammar doesn't have. Some of those conventions (clause
// heads like `| Cons` with no matching close-pipe) are genuinely undecidable
// from tokens alone. So this grammar mirrors the same split: only the truly
// unambiguous structure — top-level `: ... ;` / `type: ... ;` / ... forms and
// real paired `(...)`/`[...]` delimiters — gets a grammar rule; everything
// else is a flat `word` token, and highlights.scm classifies it by regex.

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
				$.import_definition,
				$.export_definition,
			),

		word_definition: ($) =>
			seq(":", field("name", $.word), repeat($._atom), ";"),
		type_definition: ($) =>
			seq("type:", field("name", $.word), repeat($._atom), ";"),
		extern_definition: ($) =>
			seq("extern:", field("name", $.word), repeat($._atom), ";"),
		import_definition: ($) =>
			seq("import:", field("alias", $.word), repeat($._atom), ";"),
		export_definition: ($) => seq("export:", repeat($._atom), ";"),

		// A stack effect `( ... )` and a quotation/array-type `[ ... ]` are the
		// only genuinely unambiguous, always-paired delimiters in the language;
		// everything they contain is just more atoms.
		_atom: ($) =>
			choice(
				$.word,
				$.int,
				$.float,
				$.string,
				"|",
				$.paren_group,
				$.bracket_group,
			),
		paren_group: ($) => seq("(", repeat($._atom), ")"),
		bracket_group: ($) => seq("[", repeat($._atom), "]"),

		// Base word chars exclude `; ( ) | [ ] "` and whitespace, except that a
		// mid-word `|` glues into the token when immediately followed by `>`
		// (the `S|>fi` peek-word rule in lexer.rs) — but only after at least one
		// ordinary char, so a leading `|` always stays the standalone delimiter.
		word: ($) =>
			token(
				prec(
					1,
					seq(/[^\s;()|\x5b\]"]/, repeat(choice(/[^\s;()|\x5b\]"]/, "|>"))),
				),
			),

		int: ($) => token(prec(2, /-?[0-9]+/)),
		float: ($) => token(prec(2, /-?[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?/)),

		string: ($) =>
			token(seq('"', repeat(choice(/[^"\\\n]/, seq("\\", /./))), '"')),

		// `\` starts a comment only when it stands alone as its own word (i.e.
		// is immediately followed by whitespace or EOF) — `\x` glued to more
		// text is just an ordinary (almost always erroneous) word to the real
		// lexer, not a comment.
		comment: ($) => token(prec(2, choice(seq("\\", /[ \t][^\n]*/), "\\"))),
	},
});
