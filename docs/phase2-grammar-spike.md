# Phase 2 — grammar spike: result

**Verdict: GO.** abaplint's grammar can be generated into Rust declaratively, and the generated
tables type all 33 keyword-collision probes correctly.

Pinned: `@abaplint/core` 2.120.63 (MIT). Reproduce:

```
npm pack @abaplint/core@2.120.63 && tar -xzf abaplint-core-2.120.63.tgz   # -> package/
node tools/gen-grammar/gen.cjs package <out-dir>   # writes grammar.json, report.json, grammar_tables.rs, token_kinds.txt
cp <out-dir>/grammar_tables.rs crates/abapfmt_grammar/src/tables.rs
cargo test -p abapfmt_grammar
```

## How the generator works
abaplint defines each statement/expression as combinator objects (`seq`, `alt`, `opt`, `per`, ...).
The generator loads the compiled package and walks those objects instead of parsing TypeScript, so
it does not care how a grammar file is written. Nodes are interned (6,341 unique), expressions are
referenced by index, and `ver()` gates carry the release ordinal so `Options::release` selects the
ABAP release at match time. Anything that is not a known combinator is emitted as a `custom` node
and counted as needing an override.

## Coverage
| | total | declarative | needs override |
|---|---|---|---|
| statements | 317 | 317 (100%) | 0 |
| expressions (reachable) | 269 | 269 (100%) | 0 |

Three expressions are not referenced by any statement (`SQLArithmeticOperator`, `SQLArithmetics`,
`SQLFieldListLoop`) and are not emitted. The full breakdown is `docs/grammar-spike-report.json`.

## What is **not** declarative (hand-written in later phases)
- `statement_parser.js` (378 lines): splitting into statements, `:` chains, pragma removal,
  first-word candidate lookup (`first()`, mirrored in `Stmt::first`), EXEC SQL / AMDP native SQL.
- `expand_macros.js` (275 lines): `DEFINE` macro expansion (we leave macro bodies untyped).
- `_select_reclassify.js` (169 lines): SELECT variants.
- The lexer: 31 whitespace-flavoured token kinds (`WParenLeftW`, `Dash`, `StaticArrow`, ...) that
  the grammar matches with `tok()`. Phase 3's lexer must classify them exactly as abaplint's
  `1_lexer/lexer.ts` does (white before/after the character); the probe test carries a throwaway
  version of that rule.

## Matcher
`abapfmt_grammar::match_statement` mirrors abaplint's `combi.ts` result-set semantics, including
the prio variants (first-hit / fewest-remaining), `Plus` = `seq(x, star(x))` per input,
`Expression.run` per input, `Permutation`'s shared `left` list, and `FailCombinator`/`FailStar`.
A matched token is `Keyword` (`str()` word), `Identifier` (`regex()`), or `Punct` (`tok()`), which is
exactly the classification the old plan's refiner needed.

## Probe results
`crates/abapfmt_grammar/tests/probes.rs`: 33 statements (the 27 listed in the old repo's
`2026-10-02-abaplint-token-refiner.md` plus the ones the heuristic refiner already got right),
all correct: **33/33**. Difference from the old note: `RETURN text.` is *not* `Unknown` in 2.120.63;
it matches `Return` and types `text` as an identifier, so it no longer needs the fallback.
(Template strings and the colon-chained `SELECT` forms are not covered by the throwaway lexer.)

Speed: the 33 probes take 0.25 s in a release build including thread start-up, and every statement
recompiles its regexes. Phase 3 should cache compiled regexes per grammar (341 distinct patterns)
and index statements by first word once.

## Risks / follow-ups
- **Regex dialect.** 341 patterns are JS regexes; two use negative lookahead, so the matcher uses
  `fancy-regex`. `\w` is Unicode-aware there but ASCII in JS; switch to `(?-u)` if non-ASCII
  identifiers matter.
- **Stack depth.** The grammar is deeply recursive; the test runs on a 256 MB thread. Phase 3 should
  either do the same for the formatter's parse thread or make `run` iterative.
- **Bundle size.** `tables.rs` is 225 KB of source, committed so a build needs no Node. The plan's
  "generate at build time" can still replace that later.
- **License.** Add abaplint's MIT notice to `NOTICE` (currently only "Planned") before release.
