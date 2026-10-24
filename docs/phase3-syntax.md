# Phase 3 — lexer, syntax, refiner: result

Pipeline (`abapfmt_syntax::parse`): `abapfmt_lexer` (lossless formatter tokens) → `abapfmt_grammar`
(abaplint-style lexer, statement/chain splitting, generated-grammar matcher) → each formatter word
gets KEYWORD or IDENTIFIER from the grammar token(s) inside its span (IDENTIFIER wins for fused
tokens like `lo->m(`). Words in statements the grammar cannot parse fall back to a keyword guess
using the grammar's own literal-word set (no SAP keyword list is copied).

Corpus gate (`crates/abapfmt_syntax/tests/corpus.rs`, all 1,650 non-DDL fixture snippets + samples):
7,365 / 7,420 commands (99.3%) fully matched; 992 / 36,875 words (2.7%) via fallback; no lexer
errors; 0.6 s in a release build. Remaining misses are statements newer than the pinned abaplint
(`AUTHORITY-CHECK DISABLE ...`), unusual SELECT lists (`SELECT +1 AS ...`), and macro/native SQL
(not expanded by design).

## Not done in this phase (deliberately)
- **Block nesting** (IF/LOOP/METHOD/CLASS sections for INSET): derive from `Command::statement`
  names when the INSET rule is ported (phase 5); the grammar crate does not generate abaplint's
  `3_structures`.
- **Heuristic refiner details** (operand-position cases A–F, constructor-operator checks from the
  old `token-type-refiner.ts`): only the plain keyword guess is ported; the 2.7% fallback words
  decide whether more is needed once the fixture harness (phase 4) shows failures.
- DDL / non-ABAP sections in the formatter lexer.
- Lowercase-keyword list for UPPER_AND_LOWER_CASE (`abap_true`, `sy`, ... ) arrives with that rule.
