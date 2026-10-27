# The generated grammar

abapfmt needs to tell keywords from identifiers (`READ TABLE lines INTO line INDEX index.`). It does
this with abaplint's statement and expression grammar (MIT), which is generated into Rust tables
(`crates/abapfmt_grammar/src/tables.rs`) by `tools/gen-grammar/gen.cjs`. Do not edit `tables.rs` by
hand. The weekly `refresh-grammar` workflow regenerates it and opens a pull request when a newer
`@abaplint/core` exists.

## Regenerating by hand

Replace `<version>` with an `@abaplint/core` version:

```
npm pack @abaplint/core@<version> && tar -xzf abaplint-core-<version>.tgz   # -> package/
node tools/gen-grammar/gen.cjs package <out-dir>   # writes grammar.json, report.json, grammar_tables.rs, token_kinds.txt
cp <out-dir>/grammar_tables.rs crates/abapfmt_grammar/src/tables.rs
cargo test --workspace --release
```

Then update the version in `NOTICE`. Regenerating the pinned version reproduces `tables.rs` byte for
byte. Check the new grammar with `cargo run --release -p abapfmt_syntax --example unmatched --
fixtures/samples/*.abap`, which lists the commands that are not fully matched.

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
`SQLFieldListLoop`) and are not emitted. The full breakdown is `docs/grammar-coverage.json`.

## What is **not** generated (hand-written)
- `statement_parser.js` (378 lines): splitting into statements, `:` chains, pragma removal,
  first-word candidate lookup (`first()`, mirrored in `Stmt::first`), EXEC SQL / AMDP native SQL.
- `expand_macros.js` (275 lines): `DEFINE` macro expansion (we leave macro bodies untyped).
- `_select_reclassify.js` (169 lines): SELECT variants.
- The lexer: 31 whitespace-flavoured token kinds (`WParenLeftW`, `Dash`, `StaticArrow`, ...) that
  the grammar matches with `tok()`. They are classified as abaplint's `1_lexer/lexer.ts` does
  (white before/after the character).

## Matcher
`abapfmt_grammar::match_statement` mirrors abaplint's `combi.ts` result-set semantics, including
the prio variants (first-hit / fewest-remaining), `Plus` = `seq(x, star(x))` per input,
`Expression.run` per input, `Permutation`'s shared `left` list, and `FailCombinator`/`FailStar`.
A matched token is `Keyword` (`str()` word), `Identifier` (`regex()`), or `Punct` (`tok()`).
Statements the grammar cannot parse fall back to a keyword guess (see `architecture.md`). Results
are cached by the exact tokens of a statement.

## Tests
`crates/abapfmt_grammar/tests/probes.rs` types 33 keyword-collision statements, all correctly.
`crates/abapfmt_syntax/tests/corpus.rs` parses all fixture snippets and samples: 99.3% of commands
are matched fully, and no fixture fails because of a fallback word. The remaining misses are
macros, native SQL, statements newer than the pinned abaplint (`AUTHORITY-CHECK DISABLE ...`) and
unusual SELECT lists (`SELECT +1 AS ...`).

## Notes
- **Regex dialect.** 341 patterns are JS regexes; two use negative lookahead, so the matcher uses
  `fancy-regex`. `\w` is Unicode-aware there but ASCII in JS; switch to `(?-u)` if non-ASCII
  identifiers matter.
- **Stack depth.** The grammar is deeply recursive; tests that match whole files run on a large
  thread stack.
- **Source size.** `tables.rs` is about 225 KB of source, committed so that a build needs no Node.
- **License.** abaplint's MIT notice is in `NOTICE`.
