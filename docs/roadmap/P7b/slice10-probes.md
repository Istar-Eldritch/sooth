# P7b.S10 probes — export-ambiguity / third-module shape

- Date: 2026-09-05. HEAD: `cd44b1c` (suite 3175/0 pre-probe; re-verified 3175/0
  post-revert). Binary: worktree `target/debug/sooth`, dev profile.
- Method: fixture matrix (29 dirs under `/tmp/p7bs10-probes/fixtures/`) built
  and run against the current compiler; one print-only instrumentation spike
  (P6, `S10P6` tags at the env build `src/check.rs:586`, the single-candidate
  arm `src/check/terms.rs:928-955`, and the per-word flush `src/check.rs:1047-1067`),
  fully reverted before any commit — this doc and the log below are the only
  residue.
- Discipline: this file is a VERBATIM log. Corrections go in an errata section
  at the end; existing sections are never rewritten.

---

# P7b.S10 recon probe log — export-ambiguity / third-module shape

- HEAD: cd44b1c168c48fb64b60fcef5c0ed6f74d0a741f (cd44b1c, suite 3175/0 pre-probe)
- Binary: /root/code/ordfruma/sooth-worktrees/p7b-s9/target/debug/sooth (cargo build, dev profile)
- Fixtures: /tmp/p7bs10-probes/fixtures/*

## P1 baseline (import order a,b) — 3 rebuild+run cycles

```
$ sooth build /tmp/p7bs10-probes/fixtures/p1-a-b/main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
2
exit: 0
$ sooth build /tmp/p7bs10-probes/fixtures/p1-a-b/main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
2
exit: 0
$ sooth build /tmp/p7bs10-probes/fixtures/p1-a-b/main.sth   # cycle 3
exit: 0
$ ./main   # cycle 3
2
exit: 0
```

## P1 baseline (import order b,a) — 3 rebuild+run cycles

```
$ sooth build /tmp/p7bs10-probes/fixtures/p1-b-a/main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
2
exit: 0
$ sooth build /tmp/p7bs10-probes/fixtures/p1-b-a/main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
2
exit: 0
$ sooth build /tmp/p7bs10-probes/fixtures/p1-b-a/main.sth   # cycle 3
exit: 0
$ ./main   # cycle 3
2
exit: 0
```

## P2 both a and b spell Widget[i64] eagerly — c bare Widget size

```
$ sooth build main.sth   # cycle 1
error: no overload of `Widget` in `try` (line 3) accepts these operands
  candidate: `i64`
  candidate: `i64`
exit: 1
$ sooth build main.sth   # cycle 2
error: no overload of `Widget` in `try` (line 3) accepts these operands
  candidate: `i64`
  candidate: `i64`
exit: 1
```

## P3 compat: single lib declares Widget+impl, app bare ctor

```
$ sooth build main.sth   # cycle 1
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P4 c annotates ( Widget[i64] -- i64 ) with no own header

```
$ sooth build main.sth   # cycle 1
error: unknown type `Widget` at line 3, col 9
exit: 1
```

## P8 only a spells eagerly (b bare)

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
1
exit: 0
$ sooth build main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
1
exit: 0
```

## P3a compat: lib keeps Widget-touching word private; app bare ctor

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
7
exit: 0
$ sooth build main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
7
exit: 0
```

## P3b compat: lib exports Widget + usesize; app bare ctor

```
$ sooth build main.sth   # cycle 1
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
$ sooth build main.sth   # cycle 2
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P5a: a exports mk (effect names Widget[i64]), Widget NOT exported

```
$ sooth build main.sth   # cycle 1
error: exported word `mk` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P5b: a exports Widget type; c selective-imports the type; bare ctor

```
$ sooth build main.sth   # cycle 1
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
$ sooth build main.sth   # cycle 2
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P5c: a AND b export Widget; c selective-imports from BOTH

```
$ sooth build main.sth   # cycle 1
error: duplicate import qualifier `Widget` at line 4, col 1 in /tmp/p7bs10-probes/fixtures/p5c-type-export-both/c.sth:
  qualifier `Widget` was first bound at line 3, col 1
exit: 1
```

## P5d: both export Widget; c selective-imports from a ONLY

```
$ sooth build main.sth   # cycle 1
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
$ sooth build main.sth   # cycle 2
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P7a: bare Widget in c, no eager mint anywhere, type selective-imported

```
$ sooth build main.sth   # cycle 1
error: unknown word `Widget` in `try` (line 4)
exit: 1
```

## P7b: qualified a::Widget in TERM position in c

```
$ sooth build main.sth   # cycle 1
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P7c: qualified a::Widget[i64] in TYPE position in c's signature

```
$ sooth build main.sth   # cycle 1
error: exported word `usesize` (line 4, col 3) names private type `Widget[i64]`, which is not exported
  export `Widget[i64]` too, or remove it from the effect
exit: 1
```

## P5b2: a exports ONLY type name Widget (usesize private — its signature still eagerly mints); c imports a with qualifier Widget (NOT selective — syntax probe), bare ctor

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
1
exit: 0
```

## P5e: c spells Widget[i64] in own signature (with qualifier-Widget import — NOT selective; see P5g2 for the selective form); bare ctor via mk

```
$ sooth build main.sth   # cycle 1
error: unknown type `Widget` at line 4, col 15
exit: 1
$ sooth build main.sth   # cycle 2
error: unknown type `Widget` at line 4, col 15
exit: 1
```

## P5f: literal export: Widget[i64] spelling

```
$ sooth build main.sth   # cycle 1
error: parse error: expected `;` terminating `export:`, found LBracket at line 5, col 15
exit: 1
```

## P7b2: qualified a::Widget in TERM position (a exports only type name)

```
$ sooth build main.sth   # cycle 1
error: unknown word `a::Widget` in `try` (line 3)
exit: 1
```

## P7c2: qualified a::Widget[i64] in TYPE position (a exports only type name)

```
$ sooth build main.sth   # cycle 1
error: `usesize` is not exported from module `a` at line 3, col 17
exit: 1
```

## P5g (correct pipe syntax): a exports type name; c selective-imports Widget, spells Widget[i64] + bare ctor

```
$ sooth build main.sth   # cycle 1
error: duplicate import qualifier `a` at line 3, col 1 in /tmp/p7bs10-probes/fixtures/p5g-selective-type/c.sth:
  qualifier `a` was first bound at line 2, col 1
exit: 1
$ sooth build main.sth   # cycle 2
error: duplicate import qualifier `a` at line 3, col 1 in /tmp/p7bs10-probes/fixtures/p5g-selective-type/c.sth:
  qualifier `a` was first bound at line 2, col 1
exit: 1
```

## P5h: both export Widget; c pipe-selective-imports from BOTH

```
$ sooth build main.sth   # cycle 1
error: duplicate import qualifier `a` at line 3, col 1 in /tmp/p7bs10-probes/fixtures/p5h-selective-both/c.sth:
  qualifier `a` was first bound at line 2, col 1
exit: 1
```

## P5i: both export+mint; c selective-imports from a ONLY; bare ctor

```
$ sooth build main.sth   # cycle 1
error: duplicate import qualifier `a` at line 3, col 1 in /tmp/p7bs10-probes/fixtures/p5i-selective-one-of-two/c.sth:
  qualifier `a` was first bound at line 2, col 1
exit: 1
$ sooth build main.sth   # cycle 2
error: duplicate import qualifier `a` at line 3, col 1 in /tmp/p7bs10-probes/fixtures/p5i-selective-one-of-two/c.sth:
  qualifier `a` was first bound at line 2, col 1
exit: 1
```

## P5j: both export Widget; c selective-imports NEITHER (2-mint baseline)

```
$ sooth build main.sth   # cycle 1
error: no overload of `Widget` in `try` (line 3) accepts these operands
  candidate: `i64`
  candidate: `i64`
exit: 1
$ sooth build main.sth   # cycle 2
error: no overload of `Widget` in `try` (line 3) accepts these operands
  candidate: `i64`
  candidate: `i64`
exit: 1
```

## P7c3: qualified a::Widget[i64] in TYPE position; main bare Widget

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
1
exit: 0
$ sooth build main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
1
exit: 0
```

## P5g2 (selective-only form): a exports type name; c imports a|Widget|, spells Widget[i64] + bare ctor

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
1
exit: 0
$ sooth build main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
1
exit: 0
```

## P5h2: both export Widget; c selective-imports Widget from BOTH (qualifiers a,b)

```
$ sooth build main.sth   # cycle 1
error: selective import of `Widget` from module `b` (line 3, col 19) collides with the selective import of `Widget` from module `a`
exit: 1
```

## P5i2: both export+mint; c selective-imports Widget from a ONLY

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
1
exit: 0
$ sooth build main.sth   # cycle 2
exit: 0
$ ./main   # cycle 2
1
exit: 0
```

## P6 SPIKE — G4 baseline (p1-a-b), instrumented build+check trace

```
$ sooth build main.sth (spiked binary) — stderr trace, fixture p1-a-b
S10P6 env-build: module.structs.len()=3 generic_structs=["Widget@m7", "Widget@m8"]
S10P6 env-build: structs[0] name_static=StrBuf name=StrBuf__m4 module=4
S10P6 env-build: structs[1] name_static=Stdout name=Stdout__m5 module=5
S10P6 env-build: structs[2] name_static=Widget[i64] name=Widget[i64]__m8 module=8
S10P6 env-build: env entry name=StrBuf__m4 symbol=StrBuf__m4 module=4
S10P6 env-build: env entry name=StrBuf__m4> symbol=StrBuf__m4> module=4
S10P6 env-build: env entry name=Stdout__m5 symbol=Stdout__m5 module=5
S10P6 env-build: env entry name=Stdout__m5> symbol=Stdout__m5> module=5
S10P6 env-build: env entry name=Widget symbol=Widget[i64]__m8 module=8
S10P6 env-build: env entry name=Widget> symbol=Widget[i64]__m8> module=8
S10P6 scan: word `try` candidates=1 ["try__m2@m2"] caller_span_module=0
S10P6 scan: word `.` candidates=15 [".__m1$$0@m1", ".__m1$$1@m1", ".__m1$$2@m1", ".__m1$$3@m1", ".__m1$$4@m1", ".__m1$$5@m1", ".__m1$$6@m1", ".__m1$$7@m1", ".__m1$$8@m1", ".__m1$$9@m1", ".__m1$$10@m1", ".__m1$$11@m1", ".__m1$$12@m1", ".__m1$$13@m1", ".__m1$$14@m1"] caller_span_module=0
S10P6 flush: after word main@m0 structs.len()=3
S10P6 scan: word `sys-write-str` candidates=1 ["sys-write-str__m1@m1"] caller_span_module=1
S10P6 flush: after word print-str__m1@m1 structs.len()=3
S10P6 scan: word `print-str` candidates=1 ["print-str__m1@m1"] caller_span_module=1
S10P6 flush: after word newline__m1@m1 structs.len()=3
S10P6 scan: word `print-str` candidates=1 ["print-str__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `sys-strlen` candidates=1 ["sys-strlen__m1@m1"] caller_span_module=1
S10P6 scan: word `sys-write-str` candidates=1 ["sys-write-str__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `Stdout` candidates=1 ["Stdout__m5@m5"] caller_span_module=1
S10P6 scan: word `newline` candidates=1 ["newline__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `StrBuf` candidates=1 ["StrBuf__m4@m4"] caller_span_module=1
S10P6 scan: word `g-fmt` candidates=1 ["g-fmt__m1@m1"] caller_span_module=1
S10P6 scan: word `sys-write-buf` candidates=1 ["sys-write-buf__m1@m1"] caller_span_module=1
S10P6 flush: after word print-g__m1@m1 structs.len()=3
S10P6 scan: word `print-g` candidates=1 ["print-g__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `print-g` candidates=1 ["print-g__m1@m1"] caller_span_module=1
S10P6 flush: after word .__m1@m1 structs.len()=3
S10P6 scan: word `Widget` candidates=1 ["Widget[i64]__m8@m8"] caller_span_module=2
S10P6 flush: after word try__m2@m2 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 flush: after word append-byte__m4@m4 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 flush: after word divmod10__m4@m4 structs.len()=3
S10P6 scan: word `divmod10` candidates=1 ["divmod10__m4@m4"] caller_span_module=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 flush: after word append-digits__m4@m4 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;i64__m4@m4 structs.len()=3
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;usize__m4@m4 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;isize__m4@m4 structs.len()=3
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;Bool__m4@m4 structs.len()=3
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;u8__m4@m4 structs.len()=3
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;u16__m4@m4 structs.len()=3
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;u32__m4@m4 structs.len()=3
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;u64__m4@m4 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;i8__m4@m4 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;i16__m4@m4 structs.len()=3
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-byte` candidates=1 ["append-byte__m4@m4"] caller_span_module=4
S10P6 scan: word `append-digits` candidates=1 ["append-digits__m4@m4"] caller_span_module=4
S10P6 flush: after word show;Show;4;i32__m4@m4 structs.len()=3
S10P6 scan: word `sys-write` candidates=1 ["sys-write__m5@m5"] caller_span_module=5
S10P6 flush: after word write;Write;4;Stdout__m5@m5 structs.len()=3
S10P6 scan: word `Widget` candidates=1 ["Widget[i64]__m8@m8"] caller_span_module=7
S10P6 flush: after word run__m7@m7 structs.len()=3
S10P6 flush: after word usesize__m8@m8 structs.len()=4
S10P6 scan: word `Widget` candidates=1 ["Widget[i64]__m8@m8"] caller_span_module=8
S10P6 scan: word `usesize` candidates=1 ["usesize__m8@m8"] caller_span_module=8
S10P6 flush: after word run__m8@m8 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;i8__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;i16__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;i32__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;i64__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;u8__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;u16__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;u32__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;u64__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;usize__m9@m9 structs.len()=4
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;isize__m9@m9 structs.len()=4
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;f32__m9@m9 structs.len()=4
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 flush: after word cmp;Ord;9;f64__m9@m9 structs.len()=4
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `Less` candidates=1 ["Less@m9"] caller_span_module=9
S10P6 scan: word `Greater` candidates=1 ["Greater@m9"] caller_span_module=9
S10P6 scan: word `Equal` candidates=1 ["Equal@m9"] caller_span_module=9
S10P6 scan: word `True` candidates=1 ["True@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
S10P6 scan: word `False` candidates=1 ["False@m3"] caller_span_module=9
exit: 0
$ ./main
2
```

## P6 SPIKE — P2 both-eager fixture: candidate scan + ambiguity

```
$ sooth build main.sth (spiked) — fixture p2-both-eager, Widget/run lines:
S10P6 env-build: module.structs.len()=4 generic_structs=["Widget@m7", "Widget@m8"]
S10P6 env-build: structs[0] name_static=StrBuf name=StrBuf__m4 module=4
S10P6 env-build: structs[1] name_static=Stdout name=Stdout__m5 module=5
S10P6 env-build: structs[2] name_static=Widget[i64] name=Widget[i64]__m7 module=7
S10P6 env-build: structs[3] name_static=Widget[i64] name=Widget[i64]__m8 module=8
S10P6 env-build: env entry name=Widget symbol=Widget[i64]__m7 module=7
S10P6 env-build: env entry name=Widget> symbol=Widget[i64]__m7> module=7
S10P6 env-build: env entry name=Widget symbol=Widget[i64]__m8 module=8
S10P6 env-build: env entry name=Widget> symbol=Widget[i64]__m8> module=8
S10P6 scan: word `Widget` candidates=2 ["Widget[i64]__m7@m7", "Widget[i64]__m8@m8"] caller_span_module=2
error: no overload of `Widget` in `try` (line 3) accepts these operands
exit: 0
```

## P6 SPIKE — P5i2 selective-pin fixture: tier-2 pick trace

```
$ sooth build main.sth (spiked) — fixture p5i2, Widget/run lines:
S10P6 env-build: module.structs.len()=4 generic_structs=["Widget@m7", "Widget@m8"]
S10P6 env-build: structs[0] name_static=StrBuf name=StrBuf__m4 module=4
S10P6 env-build: structs[1] name_static=Stdout name=Stdout__m5 module=5
S10P6 env-build: structs[2] name_static=Widget[i64] name=Widget[i64]__m7 module=7
S10P6 env-build: structs[3] name_static=Widget[i64] name=Widget[i64]__m8 module=8
S10P6 env-build: env entry name=Widget symbol=Widget[i64]__m7 module=7
S10P6 env-build: env entry name=Widget> symbol=Widget[i64]__m7> module=7
S10P6 env-build: env entry name=Widget symbol=Widget[i64]__m8 module=8
S10P6 env-build: env entry name=Widget> symbol=Widget[i64]__m8> module=8
S10P6 scan: word `Widget` candidates=2 ["Widget[i64]__m7@m7", "Widget[i64]__m8@m8"] caller_span_module=2
S10P6 flush: after word try__m2@m2 structs.len()=4
S10P6 flush: after word usesize__m7@m7 structs.len()=4
S10P6 flush: after word usesize__m8@m8 structs.len()=4
$ ./main
1
```

## P5k control: concrete type Point exported; app selective-imports Point and names it in an exported effect

```
$ sooth build main.sth   # cycle 1
exit: 0
$ ./main   # cycle 1
3
exit: 0
```

---

# VERDICT (P7b.S10 recon round — export-ambiguity / third-module shape)

Repo /root/code/ordfruma/sooth-worktrees/p7b-s9, HEAD cd44b1c (tree clean before and
after; P6 spike fully reverted, suite re-verified 3175 passed / 0 failed post-revert).

## Per-probe one-liners

- **P1** — G4 baseline re-confirmed on this HEAD: a+b same-shaped impls, c bare →
  builds, prints `2` deterministically, 3/3 rebuild+run cycles on BOTH import orders
  (a,b) and (b,a), exit 0.
- **P2** — BOTH modules spelling `Widget[i64]` eagerly → c's bare call sees TWO
  env candidates and the S5 accepted-ambiguity error FIRES today, deterministically
  (2/2 cycles): `error: no overload of \`Widget\` in \`try\` (line 3) accepts these
  operands` + two `candidate: \`i64\`` lines, exit 1. It names neither candidate
  module (input shapes only). Source: `no_overload_matches_error`, src/check.rs:1479-1501,
  reached from`tier_pick`'s`Ambiguous` (src/check/builtins.rs:113-141) via
  src/check/terms.rs:1017-1019.
- **P3** — compat shape works but ONLY with lib's Widget-touching words private:
  lib declares Widget+impl, app bare ctor → prints lib's constant (`7`), 2/2 cycles
  (p3a). The moment lib EXPORTS a word whose effect names `Widget[i64]`, the R18
  gate rejects it even with `export: Widget ;` present (p3b) — the gate demands an
  export entry literally spelled `Widget[i64]` (see P5f: unspellable). So the legal
  compat shape today is exactly the silent single-candidate pick of lib's mint.
- **P4** — c annotating `( Widget[i64] -- i64 )` with no own header →
  `error: unknown type \`Widget\` at line 3, col 9`, exit 1. That is the parser's
  type-NAME visibility rule (`resolve_type_or_apply` → `bare_generic_owner`
  (src/parser.rs:7132-7137: own header, else selective import, else own module) →
  `resolve_type` src/parser.rs:6434-6450), i.e. the standing cross-module
  generic-instantiation limit the roadmap hangs on P7b.S4 ("blocked behind the
  standing cross-module generic-instantiation limit (P7b.S4)",
  docs/roadmap/P7b-higher-kinded-types.md:166) — NOT S4's declaring-module mint
  keying (S4's fix, never reached) and NOT the R18 export gate.
- **P5** — sanctioned cross-module type(+impl) sharing channels today, exhaustive:
  (i) `export: Widget ;` (generic header BASE name; legal) + consumer selective
  import `import: self::a | Widget | ;` → consumer can spell `Widget[i64]` in its
  own signatures and its bare ctor then pins the exporting module's candidate via
  S5 tier 2 (p5g2 prints 1, 2/2; p5i2 with BOTH modules minting still picks a's
  via tier 2, prints 1, 2/2). (ii) qualified type spelling `a::Widget[i64]` in type
  positions with a plain import + exported base name (p7c3 prints 1, 2/2).
  (iii) consumer declares its own same-named header (the G1 twin shape).
  BLOCKED channels: exporting any word whose effect names `Widget[i64]` (exact text:
  `error: exported word \`mk\` (line 4, col 3) names private type \`Widget[i64]\`,
  which is not exported\` + `export \`Widget[i64]\` too, or remove it from the
  effect`, declarations.rs:796-806) — and the remedy is UNSATISFIABLE for
  instantiations:`export: Widget[i64]`is a parse error (`expected \`;\`
  terminating \`export:\`, found LBracket`), while`export: Widget ;` does not
  satisfy the gate (gate matches the Type's carried rendering `Widget[i64]`).
  Control P5k: for a CONCRETE type the remedy IS satisfiable (export`Point`,
  selective import, exported effect naming`Point`→ builds, prints 3). Two
  selective imports of one name collide (`error: selective import of \`Widget\`
  from module \`b\` ... collides with ... module \`a\``, p5h2). So: a legit
  sharing path EXISTS (type-name export + selective/qualified spelling) and any
  S10 policy must keep it working — it currently works by tier-2 pinning, not by
  the silent pick.
- **P6** — SPIKE (reverted): at c's env build, `module.structs` holds ONLY
  parse-time mints (b's `Widget[i64]@m8`; a's does not exist yet) and
  `module.generic_structs` ALREADY holds both headers (`["Widget@m7","Widget@m8"]`);
  c's bare call scan sees `candidates=1 ["Widget[i64]__m8@m8"]` at the
  single-candidate arm; a's mid-check mint lands in `module.structs` only during
  a's own word-loop flushes (len 3→4 well after c's scan) — env is built ONCE
  before any body is checked, so a's mint NEVER precedes c's env build. With both
  modules eager (P2 trace), env holds both mints and c's scan sees candidates=2 →
  the ambiguity error. Silent-pick site: the single-candidate arm's grounding
  `Ok(None)` path (`find_struct(header_name, caller_module)` else-branch,
  src/check/terms.rs:1516-1523 region) — caller has no header, borrowed mint used
  unchanged.
- **P7** — bare `Widget` resolves at word level through the env overload table
  (ctor candidates); with NO env entry (no mint anywhere) it is
  `error: unknown word \`Widget\` in \`try\` (line 4)` (p7a). Qualified ctor in
  TERM position does NOT exist: `a::Widget` → `error: unknown word \`a::Widget\`
  in \`try\` (line 3)` (p7b2, matching S9's reviewer observation and S4's m4
  remedy-spelling hole). Qualified TYPE position `a::Widget[i64]` DOES exist and
  works (p7c3).
- **P8** — determinism-brittleness witness re-confirmed: only a eager (b bare) →
  c prints `1` (a's constant), 2/2 cycles. Today's output for c is decided by
  which unrelated module happens to spell the instantiation.

## Synthesis

**(a) Silent-pick shapes today.** Exactly the shapes where c's env holds exactly
ONE `Widget` candidate at c's scan and c declares no own header: (1) G4/P1 (one
eager minter program-wide — silent deterministic pick of the minter, P1+P8 show
the minter identity alone flips the answer 2↔1); (2) the single-lib compat shape
(P3a/P5b2 — benign while only one header/impl exists program-wide, but the SAME
mechanism, not a special case). Not silent today: both-mint eager (P2/P5j —
existing ambiguity error), double selective import (P5h2 — collision), unexported
type named cross-module (P4 — unknown type), export-gate trips (P5a/P3b).

**(b) Reuse select_overload's accepted-ambiguity error?** The existing error
(`no_overload_matches_error`) is REACHABLE for this name shape (P2 fires it) but
only when the candidate list has 2+ entries — its text lists operand shapes and
duplicates identical lines (`candidate: \`i64\`` twice), naming no modules. The
G4 shape never reaches it (1 candidate). Reusing it verbatim therefore requires
making the 2-candidate state observable at the call (populating env per-header or
equivalent); otherwise a NEW located message is needed. Either way the current
text does not satisfy the working ruling's "naming the candidate modules + call
site" — the call-site word/line IS in the message, the candidate modules are NOT;
extending the message (or a new one) is required. Note also `tier_pick`'s lone
survivor comment (builtins.rs:118-127): the tier policy deliberately never
errors on a single candidate, so the policy cannot simply lower a threshold
inside`tier_pick` without re-litigating that ruling.

**(c) Natural policy layer (from P6).** The information needed converges at the
single-candidate arm in terms.rs (the grounding call): caller span.module, the
single foreign candidate, and — via `ctx.generics()` / `module.generic_structs` —
the whole-program set of same-named headers, which is COMPLETE at env-build time
(P6: both headers visible before any body is checked; env is frozen at build
time, so per-header candidate lists cannot come from env without changing env
construction). `select_overload`/`tier_pick` cannot see the latent ambiguity (1
candidate by construction). Word-resolution itself (env lookup) has the same
thin-candidate problem. So the measured facts place the detectable point at the
candidate-scan/grounding layer, with header provenance read from the generic
registry — not at select_overload, and not at env build (which has no call site
to locate an error at).

**(d) Compat constraints.** The policy MUST keep working: (1) P3a's single-lib
shape (lib private words, app bare ctor) — one header program-wide, so any rule
keyed on "≥2 same-named headers program-wide + caller has none + single env
candidate" leaves it untouched, while a blanket "bare ctor of a foreign mint is
an error" rule would break it; (2) P5g2/P5i2/P7c3's sanctioned channels
(exported type name + selective import; qualified spelling) — note P5i2 is
tier-2 pinning of a 2-candidate list, which already errors-or-pins correctly and
must not be re-error; (3) own-header consumers (G1 twins) — S9's R1.1a grounding.
The gate's unsatisfiable instantiation remedy (P5a/P5f) is a pre-existing,
independently-recorded wart, not something the S10 policy must fix — but it means
"export the word over the type" is NOT a workaround the policy may assume.

**(e) Remedy surface existing today.** For a c facing a/b's same-named types:
declare an own header (grounds at it, S9); selective type import
`import: self::a | Widget | ;` after `export: Widget ;` in a (enables signature
spellings AND tier-2 ctor pinning); qualified type spelling `a::Widget[i64]`
(plain import + export). NOT available: qualified ctor term `a::Widget`
(unknown word); `export: Widget[i64]` (parse error); exporting any word whose
effect names the instantiation (gate, unsatisfiable remedy for instantiations);
double selective import (collision). "Qualify with the module" is therefore a
TYPE-position remedy only; a ctor-position remedy would need new syntax.

## Corrections appendix — pre-implementation review round (2026-09-05)

A three-reviewer review round over the spec/brief/paper-tests/probes suite
(base `a9eca84`) returned BLOCK on all three lanes. The maintainer ruled two
open decisions (interview, 2026-09-05); this appendix records the additional
measurements a fix pass made while revising the spec suite to match. The
probe log above stays verbatim; nothing here edits it, only supplements it.

**G4 inversion.** S9's own golden
`third_module_bare_caller_dispatches_the_single_shared_env_instantiation`
(`tests/phase7b_slice9.rs`) asserts exactly the silent outputs (`2`/`1`) GA/GB
replace with a located error, for both import orders and both minter
placements. REQ-6 in the revised spec makes this explicit: this one S9
golden is retired/rewritten in Phase 1; every other S9 golden stays
byte-unchanged.

**Site citation correction.** The silent-pick fall-through is
`src/check/terms.rs:1507-1509` (the `find_struct(header_name, caller_module)`
→ `None` arm in `bare_generated_word_own_module_grounding`), not the
`1516-1523` region an earlier draft cited (that range is the *success* path,
taken when the caller does have its own header).

**The program-wide predicate is unsound; scope it to reachability.** Two new
fixtures, built to probe R1's original wording ("≥2 same-named headers
declared by ≥2 distinct modules, none of them `m`", with no reachability
filter):

- `sel1` (two reachable headers `a`/`b`; only `a` mints; `c` selectively
  imports `a`'s `Widget`) — measured: builds, prints `1`, exit 0. The
  original program-wide predicate would have flagged this as ambiguous
  (2 headers exist, neither is `c`'s), which would newly reject a program
  that resolves correctly and unambiguously today.
- `overfire` (two headers exist program-wide, `lib` and `z`; the caller
  `app` imports only `lib`, never `z` — `z` is imported only by `main`, a
  different module) — measured: builds, prints `7`, exit 0. Same problem: a
  program-wide count would flag this too, even though `z` is entirely
  unreachable from `app`'s own imports.

Both are now goldens (GI, GH respectively) pinning that the header count is
scoped to headers reachable through the caller's own `ModuleInfo.imports`/
`.selective` (R1), not a program-wide count.

**A bare "selective import exists" exemption is separately unsound —
measured, not assumed.** A third fixture (`/tmp/r2`, now paper-tests' `GK` /
`p9-mismatched-selective`): two reachable headers `a`/`b`; only `b` ever
eagerly mints; `c` selectively imports **`a`'s** `Widget` (`import: self::a
| Widget | ;`) plus a plain import of `b`. Measured: builds, **silently
prints `2`** (`b`'s impl), exit 0 — `c`'s own selective import named `a`, but
the checker hands it `b`'s value anyway, undetected. Root cause: the
single-candidate fall-through never consults `ModuleInfo.selective` at all
today (env is program-wide and import-blind at this arm); a naive
implementation of "the caller explicitly resolved this name" as "a selective
import of this name exists" would exempt this case from the new error,
silently reproducing the exact defect class S10 exists to close. The revised
R1 exemption 4 requires the *selected* module to equal the *actual sole
candidate's* owning module; when they disagree (as here), the exemption does
not apply and the general rule fires. GK pins this: before, silent `2`;
after, the same located error as GA/GB.

**Qualified type spelling never cures the single-candidate shape — confirmed
by two further measurements.** (1) `/tmp/r3`: `a.sth` bare (no eager mint of
its own), `b.sth` eager via its own `usesize` signature, `c.sth` writing
`a::Widget[i64]` in *its own* signature. Measured:
`error: no overload of \`Widget\` in \`main\` (line 3) accepts these
operands` + two `candidate: \`i64\`` lines — the qualified signature reference
is itself a second parse-time eager mint (of `a`), pushing the call into the
**pre-existing, unchanged** 2-candidate arm (GC's error), not a new one and
not a cure. (2) `qual2`: a variant entangling two call sites (`main`'s own
bare `Widget` ctor call, and `c`'s separate qualified-signature type
reference) — measured prints `1` today, but does not cleanly witness any
single exemption (see paper-tests' "measured but not adopted" section) and
was not adopted as a golden. Together these are the evidence behind R5's
decision to drop qualified spelling from the remedy note entirely, rather
than listing it as a working (if type-position-only) remedy.

**`own_header_still_grounded_first`, re-confirmed.** `/tmp/r1`: `c` declares
its own `Widget['T]` header and its own `impl: Sized for Widget` (constant 9)
in addition to `a`/`b`'s headers. Measured: prints `9`, exit 0 — S9's R1.1a
own-header grounding fires regardless of how many foreign headers exist.
Not a new golden (S9's own suite already covers this mechanism); recorded
here as a boundary re-check.

**Fixture correction: GD.** `p3-single-lib` (lib exports `usesize`) trips the
R18 export gate before ever reaching the ctor-call check at all
(`error: exported word \`usesize\` ... names private type \`Widget[i64]\`,
which is not exported`) — it never was a legal single-header witness. GD's
fixture is corrected to `p3a-single-lib-private` (same shape, `usesize`
kept private), which measures `7`, exit 0, as the spec always claimed.

**Naming mechanism, resolved by inspection (no fixture needed).**
`ModuleInfo` (`src/ast.rs:175-189`) carries no canonical module-name field —
only `imports: HashMap<String, u32>` (qualifier → target) and
`selective: HashMap<String, u32>` (bare name → target, merging explicit
`| name |` clauses and `*` wildcard per-export desugaring, confirmed by
reading `driver.rs`'s import-assembly pass). Every existing diagnostic that
names a foreign module (`declarations.rs:974`, `:988`, `word_families.rs:1336`)
renders the *caller's own* qualifier, not a canonical name. S10's new
diagnostic follows the same precedent (R4) and sorts the collected qualifiers
lexicographically for import-order-independent determinism (confirmed against
`p1-a-b` vs `p1-b-a`, which reach the fall-through with the identical
reachable set `{a, b}` regardless of import order).

**Sequencing (OQ-2), resolved by maintainer ruling, not measurement.**
Interview, 2026-09-05: S10 is implemented in parallel with P7b.S6 (both
branch from base `a9eca84`), not strictly before or after. Landing rule: if
S6's own probes demand a check-stage grounding change in `terms.rs`, or the
two slices' changes interact at merge time, S10's checker change lands first
and S6 rebases onto it. Recorded in the spec's REQ-8 and the brief's OQ-2.

## Corrections appendix — round-2 review (2026-09-05)

A second review round over the round-1-corrected spec suite (commit
`e9b60d8`) returned fix verdict partial/BLOCK: every round-1 finding
resolved, but the revision itself introduced one P0 and five P1s, all
measured against fresh fixtures. This appendix records what changed; the
probe log and the round-1 appendix above stay verbatim.

**P0 — exemption 4's match test was wired to the wrong datum, two facets.**
(a) `struct_instantiation_of`'s second component (`owning_module`) is the
candidate's *instantiating* module (`ast.rs:2598`'s `Generic::module` doc
comment: "the third component of `struct_keys`... captured at the naming
site"), not its *declaring* module (`guard.structs[gi].module`,
`GenericStructDecl.module`) — the round-1 spec's R2 conflated the two. Every
existing golden's candidate happens to be instantiated inside its own
declaring module, so no golden's outcome changed; the distinction is
correctness-critical for GL below, where they differ. (b) **Hub over-fire**,
measured (`/tmp/s10rev/hub`, now paper-tests' GL): `h.sth` re-exports `a`'s
`Widget` (`import: self::a | Widget | ; export: Widget ;`, no header of its
own); `c.sth` selectively imports `Widget` from `h`, plus plain imports of
`a` and `b` — builds, prints `1`, exit 0 today. `c`'s raw
`ModuleInfo.selective` value for `Widget` is `h` (the import target), not
`a` (the declaring module) — comparing the raw value directly against the
candidate's declaring module would mismatch (`h` ≠ `a`) and wrongly error a
correctly-resolving program. Fix: resolve the selective target through the
same hop-walk the checker already runs for a type reference in an effect
signature (`resolve_type_export_origins`/`walk_type_export_origin`,
`src/driver.rs:364`/`:407`) before comparing. GL pins the fixed behaviour
(unchanged, `1`, exit 0, both before and after).

**P1 — exemption 2 kept a silent mis-dispatch when the sole candidate's
owner is unreachable.** Measured (`/tmp/s10rev/under`, now paper-tests' GM):
`lib` and `z` both declare their own `Widget` header; only `z` ever eagerly
mints; `app` imports only `lib`, declares no header of its own, and
bare-calls `Widget size` — prints `9` (`z`'s impl), exit 0, even though `app`
never imports `z` in any form. Under the round-1 exemption 2
("≤1 reachable header", full stop), this stayed silently exempt — exactly
the defect class S10 exists to close, reappearing through the header-*count*
check alone not covering "is the *actual* candidate's module one the caller
can see at all". Fix: exemption 2 additionally requires the sole candidate's
declaring module to itself be reachable from `m`; when it is not (as here),
the rule fires regardless of the reachable-header count. Re-measured GD, GH,
GI unaffected by the tightening (their sole candidate's declaring module was
already reachable in every case); GM is the new regression pin. `app` has no
qualifier for `z` at all (never imported, not even via a wildcard), so the
message names it structurally — the same fallback the existing
`drop`-visibility diagnostic already uses for exactly this gap
(`word_families.rs:1319-1340`'s qualifier lookup and its `None` arm).

**P1 — guard ordering was unspecified.** Round-1's R2 cited the
`terms.rs:1507-1509` fall-through as the emission site, but that fall-through
returns `Ok(None)` immediately — *before* the candidate-identity check
(`generated_word_entry` + `key`/`symbol` match, `:1531-1537`) ever runs, since
that check today sits gated behind an own header being found. Emitting
directly at `:1507-1509` would newly reject the "ordinary user word whose
output happens to be another module's instantiation" shape `:1531-1537`
exists to protect. Fix: the new check fires only after the candidate survives
both `:1504` and `:1531-1537`; the implementing phase restructures the
function's control flow accordingly and updates the `:1492-1497` "the order
is free" comment, which no longer holds once an arm can return `Err(...)`.

**P1 — REQ-8 missed a second stale roadmap sentence.**
`docs/roadmap/P7b-higher-kinded-types.md:241-244` (the dispatch-mechanism
sentence, "A third-module bare caller ... dispatches deterministically on
the single instantiation minted into the shared whole-program env") is
distinct from the trailing Residual sentence (`:244-247`) and also goes false
once S10 lands. Fix: REQ-8, Phase-2's changes, and the JSON now name both
sentences explicitly.

**P1 — GI was missing from several acceptance-criteria enumerations.** GI
(`sel1`, the only positive exemption-4 witness in round 1) was present in
the goldens table and JSON `goldens[]` but absent from REQ-4, the REQ-4
phase-map row, the Phase-1 goldens/exit prose, the JSON Phase-1 exit text,
and two enumerations in the brief. Fixed everywhere goldens are enumerated,
alongside the new GL and GM.

**P1 — remedy 2 was a dead end in GA's own shape.** Applying "selectively
import the module whose Widget you want" to GA's fixture is exactly GK: `c`
selects `a`, `b` mints, still errors. Measured cure
(`/tmp/s10rev/rem2sig`): `c` selectively imports `a`'s `Widget` (as GK does)
**and** additionally writes `: mk ( i64 -- Widget[i64] ) Widget ;`, calling
`mk` instead of constructing `Widget` bare inline — builds, prints `1`, exit
0. `mk`'s own signature spells `Widget[i64]` explicitly, making `c` itself
the *instantiating* module for that construction, so `owning_module ==
caller_module` short-circuits at `:1504` before any borrow or ambiguity
check runs — `c`'s selective import of `a`'s `Widget` is what lets the bare,
unqualified signature reference resolve to `a`'s header at all. R5's remedy
2 is now a two-part cure: selective import alone cures only when the
selected module is the sole minter (or tier-2-pinned at the multi-candidate
arm); otherwise, additionally spelling the type in an own-signature
intermediate word cures by sidestepping the borrow entirely.

**P2 — miscited precedent.** `declarations.rs:969` (a different helper's doc
comment, `selective_source_phrase`) was cited for "its wildcard-imported
module" — the actual site is `declarations.rs:989`
(`selective_not_exported_error`'s `None` arm). Corrected throughout.

**P2 — "import closure" used in two conflicting senses.** The operative,
one-hop predicate (the caller's own `imports`/`selective` maps) and the
program-wide, whole-file-closure sense were both called "the caller's own
import closure" in places, despite meaning different things (and despite
`engine.rs:1130`'s own `ctx.modules` field doc comment also saying "the
import closure's per-module data" for the one-hop sense — the source's own
usage isn't wrong, but reusing the same word for the *other*, program-wide
sense within this spec's own prose was the ambiguity). Renamed the one-hop
predicate's every occurrence to "the caller's own import set (its own
imports and selective imports, one hop)"; the two remaining program-wide
mentions (GH's rationale, in the spec and the paper-tests) now read
"elsewhere, program-wide" instead of "elsewhere in the closure".

## Corrections appendix — round-3 review (2026-09-05)

A third review round (two lanes: mechanical, policy) over the round-2-corrected
spec suite (commit `56864d1`) confirmed all eight round-2 findings resolved,
the restructure behaviour-preserving, GL/GM byte-faithful, and the 13/9
name sync clean — mechanical lane: complete/OK-with-notes; policy lane:
complete/BLOCK on two new P1 predicate holes, plus P2 batch findings. This
appendix records what changed; the probe log and the round-1/round-2
appendices above stay verbatim.

**P1 — "reachable" was defined twice, and the two definitions diverged
(hub-unaware).** R1's exemption 2 (round-2 text) defined the reachable set
as `imports ∪ selective` target values; a separate paragraph in R2
("Everything else") and brief's own mirror instead assigned `imports` alone
to reachability and `selective` only to exemption 4's match test — an
internal fork paper-tests had already avoided (its own unit sketch already
read `imports`/`selective` together). Two measured fixtures exposed this as
load-bearing, not merely cosmetic:

- `/tmp/s10r3/selother` (now paper-tests' GO): `d` declares and mints
  `Widget`, and separately declares `Gadget`; `c` selectively imports only
  `Gadget` from `d` (`import: self::d | Gadget | ;` — a *different* name
  than the surface name under check) and bare-calls `Widget`. Measured:
  builds, prints `4`, exit 0. Under the union reading `d` stays reachable
  (it is a raw selective target, regardless of which name was selected) and
  the fixture stays legal, matching measurement; under the drifted
  imports-only reading, `c`'s raw imports set is empty (its only import
  statements are a wildcard of `f` and the selective import of `d` — neither
  populates `ModuleInfo.imports`), so `d` would be wrongly treated as
  unreachable and the fixture would wrongly error.
- `/tmp/s10r3/hubq` (now paper-tests' GN): `a` declares, mints, and exports
  `Widget`; `h` re-exports it with no header of its own; `c` writes a
  **plain** `import: self::h ;` (no selective clause at all) and bare-calls
  `Widget`. Measured: builds, prints `1`, exit 0 — only one header exists
  program-wide, nothing to mis-dispatch to. Under the round-2 tightening
  (exemption 2 requiring the sole candidate's declaring module to be
  reachable), `c`'s raw reachable set is `{h}`, `h` has no header, so `a`
  (the sole candidate's declaring module) is NOT in the raw reachable set at
  all — the fixture is newly, wrongly errored, exposing that reachability
  itself (not just exemption 4) needed the hub-chain walk.

Fix (maintainer-consistent completion of D1's reachability principle): the
reachable set is the caller's raw import set (`imports ∪ selective` target
values, name-independent — confirmed sound by GO) **plus**, for every module
in that raw set, whatever module the export-origin walk resolves the
surface name to when started there (the same walk exemption 4 uses, over the
generic header registry — confirmed sound by GN). Re-verified GD, GH, GE, GF,
GI, GJ, GM are unaffected by this widening (each fixture's sole candidate's
declaring module was already directly reachable, or already correctly
unreachable, under the narrower definition too). R1's exemption 2, R2's data
paragraph, and brief's mirror now state this as a single, non-forking
definition.

**P1 — a `*` wildcard import must not grant exemption 4.**
`driver.rs:568-600` desugars a real-target wildcard into a `selective_map`
entry for every one of the target's exported names — indistinguishable, in
the raw `ModuleInfo.selective: HashMap<String, u32>`, from a named
`| name |` selective import. Measured (`/tmp/s10r3/wild`, now paper-tests'
GP): `a` and `b` both declare `Widget`; only `b` mints; `c` writes
`import: self::a ; import: self::b * ;` and bare-calls `Widget` — builds,
silently prints `2`, exit 0, today. The round-2 predicate's exemption 4
(reading raw `ModuleInfo.selective` alone) would treat `b`'s wildcard-derived
entry as if `c` had explicitly named `Widget` from `b`, silently exempting
this case — GK's exact defect class re-entering through the desugar, and
the round-2 delta had in fact *removed* an R1 sentence (present in an
earlier draft) that flagged this gap, while R2's own data-source paragraph
still folded wildcard entries into the same exemption-4 read.

Fix (completion of D1's explicit-resolution principle, matching the cited
precedent `declarations.rs:972-976`/`:989`, which already refuses to treat a
wildcard-desugared entry as a named import — `selective_source_phrase`'s
`None` arm renders it as "wildcard import of `{name}`", never as "selective
import ... from module"): exemption 4 requires a **named** selective import
specifically; a wildcard-desugared entry never counts, however identically
it populates the same map. Since the raw `ModuleInfo.selective` cannot
itself carry this distinction, the implementing phase threads the
already-computed `SelectiveName.qualifier` field (`Some` for named, `None`
for wildcard — computed once, at assembly time, by the same driver code that
builds `selective_map`, but today discarded after `check_selective_imports`
runs) through to `Ctx`. Reachability (exemption 2) is unaffected by this
exclusion — a wildcard-reached module still counts there, confirmed by GP
itself (`b` is one of its two reachable headers regardless, which is why
the fixture reaches the general rule rather than exemption 2's
≤1-reachable-header case).

**P1/P2 — the `type_origin`/`walk_type_export_origin` reuse claim was false
for generic headers, and the fallback alternative was a dead end.** Generic
headers are **not** in the concrete type-export registries
`resolve_type_export_origins`/`walk_type_export_origin` walk
(`src/driver.rs:364`/`:407`) at all: `parser.rs:81-83` deliberately excludes
a generic `type:` header from the concrete struct/enum scan
(`if header_is_generic(...) { continue; }` — it lives in
`module.generic_structs`/`generic_enums` instead), and
`resolve_type_export_origins`'s own `declared_types` builder
(`driver.rs:373-384`) iterates only `StructDecl`/`EnumDecl`. Measured
(`/tmp/s10r3/hubtype`, built specifically to probe this): `d.sth` writes
`idw ( Widget[i64] -- Widget[i64] )`, a *type-position* reference to
`Widget` reached only through a hub (`h`, itself re-exporting `a`'s
`Widget`) — exactly the shape the round-2 spec claimed the existing walk
already covers. Measured result: `error: unknown type \`Widget\`` — the
EXISTING mechanism cannot see a generic header through a hub either, so the
round-2 claim ("S10 reuses the same hub-hop the checker already performs
for a type reference in an effect signature") was false for this shape. The
round-2 spec's stated fallback — "if simpler, thread the already-computed
`type_origin` table (`driver.rs:651`) through to `Ctx`" — is for the same
reason unusable: `type_origin` is built from the same concrete-only
`declared_types`, so it never has an entry for a generic header reached
through a hub; an implementer choosing this alternative would find no
resolution for GL at all and fail that golden.

Fix: delete the `type_origin`-threading alternative everywhere it appears
(spec, brief, paper-tests); the implementing phase builds a walk that is
**structurally the same as the checker's existing one, but over the generic
header registry** (`ctx.generics().structs`) instead of `StructDecl`/
`EnumDecl` — a new, small function, not a call into `walk_type_export_origin`
itself. Its ingredients (`ctx.generics().structs`, `ctx.modules`'s
`selective`/`imports` maps) are already reachable from `Ctx`, so this
remains no new registry. Re-worded the provenance claim throughout to
"structurally the same walk the checker runs for concrete type names, but
over the generic header registry".

**P2 batch.**

1. **False "declaring and instantiating coincide in every golden" claim.**
   `p7c3` (GF) has `c` mint `a`'s header via `c`'s own
   `: try ( a::Widget[i64] -- i64 ) size ;`, and `p5g2` (GE) has `c` mint it
   via `c`'s own `: mk ( i64 -- Widget[i64] ) Widget ;` — both declaring `a`,
   instantiating `c`, the exact divergence the round-2 claim said no golden
   exercised. The *conclusion* (this distinction changes no current
   golden's outcome) survives, but for a different reason: GE/GF are
   exactly the fixtures where declaring and instantiating diverge, and
   there the divergence is moot, because `c` (the instantiating module) is
   also the *caller*, so `owning_module == caller_module` and the call
   exits at `terms.rs:1504` before exemption 2 or 4 is ever reached.
   Reworded R2's claim to state the corrected, narrower reasoning
   ("wherever exemption 2/4 actually runs, the two coincide"), so a future
   unit test for this distinction is not built from a fixture (GE/GF) that
   in fact never reaches the code being tested.
2. **GE's (and GF's) stated mechanism was wrong.** Both golden rows read
   "exemption 2 (≤1 reachable header) fires" / "no second header exists";
   the actual mechanism is the `:1504` own-instantiating-module
   short-circuit, unrelated to exemption 2 or to how many headers exist.
   Relabeled both rows (spec, paper-tests, brief) to pin the `:1504` arm
   explicitly, and moved GE/GF out of REQ-1's exemption-2 traces
   accordingly (REQ-4's compat-pins list still includes them, since the
   golden's *behaviour* — unchanged output — is correct; only the
   *mechanism* attribution was wrong).
3. **The origin walk's `None` arm was unruled.** R2 described the walk's
   success path but not its failure path. Ruled: when the walk cannot
   resolve (cycle or dead end, `driver.rs:411-427`'s loop shape), the
   starting module contributes nothing — exemption 4 does not apply on its
   account, and it adds nothing to reachability's walk-extension either;
   the general rule (or a different exemption, or a different reachable
   module) decides.
4. **GM's draft message wrongly said "ambiguous".** GM's shape is a reach
   failure (exactly one env candidate exists; the problem is that its
   declaring module is unreachable, not that several candidates compete).
   Reworded R5's GM draft contract from "... is ambiguous: the only
   `Widget[i64]` instantiation in scope belongs to a module ... does not
   import" to "... is unresolved: the only `Widget[i64]` instantiation in
   scope is declared in a module ... does not import" — measure-then-pin
   still governs the exact rendered bytes; only the contract's own claim
   about what kind of problem this is was corrected.
