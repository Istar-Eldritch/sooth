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
