# abapfmt — Rust ABAP formatter (new repo)

## Context
The abap-cleaner VS Code port (TS; 98 rules, 11 migrated, 663/2,164 upstream fixtures) became too
complicated: ~60 rules rewrite code and drag in semantic helpers (ClassInfo, VariableInfo,
LogicalExpression), rule-order coupling and a line-diff adapter. Pivot to **abapfmt**: a Rust
formatter that changes only whitespace, line layout and letter case — never tokens. It adopts
abap-cleaner's formatting options where sensible and applies Biome/oxfmt engineering. The old repo
is left as is; abapfmt starts in a **new repo**, copying only fixtures and reference material.

## Decisions (from user)
- **Layout:** preserve user line breaks (abap-cleaner model) + gofmt-style reflow of a statement
  only when it exceeds `lineWidth`. `lineWidth` is configurable; `"off"` = behave exactly like
  abap-cleaner (only the per-rule max-line-length options of e.g. ALIGN_PARAMETERS apply).
  Default: 120 (Clean ABAP); rows of tables in tabular style may be longer (`tabularLineWidth`, 160).
- **Keyword typing:** abaplint statement/expression grammar (MIT), **generated** into Rust by a
  build-time tool from a pinned abaplint version; hand-written overrides for non-declarative files.
  New ABAP syntax = bump pin + regenerate. Unparseable statement → heuristic typing fallback
  (ported abap-cleaner TokenTypeRefiner), never a failure.
- **Editor:** native CLI + `abapfmt lsp` (formatting-only: documentFormatting +
  documentRangeFormatting over stdio) + thin VS Code extension via `vscode-languageclient`, so the
  built-in Format Document / Selection / format-on-save (incl. `modifications` mode) use it.
- **Repo:** new repo (sibling dir `C:\Users\Work\Downloads\abapfmt`), not this one.
- No semantic analysis, no caches, no language server features beyond formatting.
  `#![forbid(unsafe_code)]` in every crate (carried over from earlier user requirement).

## Rule scope
- **Keep:** EMPTY_LINES_WITHIN_METHODS / OUTSIDE_METHODS / IN_CLASS_DEFINITION; SPACES group
  (CLOSING_BRACKETS_POSITION, SPACES_IN_EMPTY_BRACKETS, SPACE_BEFORE_PERIOD,
  SPACE_AROUND_COMMENT_SIGN, SPACE_AROUND_TEXT_LITERAL, NEEDLESS_SPACES); ONE_COMMAND_PER_LINE;
  INSET; UPPER_AND_LOWER_CASE; all 17 ALIGN_*; DDL_EMPTY_LINES_*, DDL_SPACES_*, DDL_POSITION_*,
  DDL_ALIGN_*, DDL_ANNO_LAYOUT.
- **Drop:** DECLARATIONS, COMMANDS, SYNTAX groups (token rewrites), CHAIN_OF_ONE,
  DECLARATION_CHAIN, DDL_ANNO_NESTING, DDL_TYPO, CAMEL_CASE_NAME, DDL_CAMEL_CASE_NAME.
- Borderline, decide per rule in its phase: PRAGMA_POSITION (moves tokens), ALIGN_ABAP_DOC.
- Rule option names/defaults: upstream `Config*Value` names (as already mapped in
  `packages/core/src/engine/rules/**` of the old repo).

## Architecture (Biome/oxfmt lessons applied)
Cargo workspace:
- `abapfmt_lexer` — lossless tokenizer into a flat token arena (text spans + trivia: spaces, tabs,
  NBSP, line breaks, comments, pragmas, string templates, `:`/`,` chains). Keyword text interned.
- `abapfmt_grammar` — generated from abaplint `src/abap/2_statements/{statements,expressions}`
  by `xtask gen-grammar` (Node + TS compiler API → Rust combinator tables); `ver()` gating by
  configured ABAP release. Output: per-token KEYWORD/IDENTIFIER + expression node spans.
- `abapfmt_syntax` — statements, chains, block nesting (METHOD/IF/LOOP/…), bracket tree;
  bogus-statement nodes for anything unparsed (printed verbatim, Biome-style fault tolerance).
- `abapfmt_formatter` — one parse per file; rules run in upstream RuleID order over the shared
  structure, each with a statement-kind prefilter; they edit trivia/case only. Output = `TextEdit`s
  (untouched bytes preserved; no line-diff layer). Optional reflow pass when `lineWidth` set.
- `abapfmt_config` — `abapfmt.json`/`.toml` with schema, nearest-config lookup, plus `.cfj`
  (abap-cleaner profile) importer that ignores dropped rules with a warning.
- `abapfmt_cli` — `format`, `--check`, `--write`, stdin, globs, rayon file parallelism.
- `abapfmt_lsp` — `tower-lsp`/`lsp-server`, formatting + range formatting only.
- `editors/vscode` — TS extension: contributes `abap` language id (`*.abap`, `*.acds`?) if absent,
  launches bundled per-platform binary, `editor.defaultFormatter` docs.

**Safety invariant (tested on every fixture and sample):** token stream after formatting equals
before, compared case-insensitively. Plus idempotence: `fmt(fmt(x)) == fmt(x)`.

## Phases (each becomes its own plan)
1. **Bootstrap repo:** workspace, CI-less local scripts, licenses (Apache-2.0 NOTICE for
   SAP-derived options/fixtures, MIT notice for abaplint-generated grammar). Copy from old repo:
   `packages/core/test/engine/fixtures/rules/**` (keep-scope JSONs only, language-neutral format:
   `src`/`exp`/`config`/`flags.wrappers`), `cleanup-sample.abap/.acds`, `tools/import-java-fixtures.mjs`,
   and pin upstream abap-cleaner by git ref instead of copying `reference-eclipse-plugin/` (18 MB).
2. **Grammar spike:** run the generator over abaplint; report % of files translated
   declaratively vs needing overrides; validate on the 50 probe statements from
   `docs/superpowers/plans/2026-10-02-abaplint-token-refiner.md` (old repo). Go/no-go gate.
3. **Lexer + syntax + heuristic refiner:** port from old repo `packages/core/src/engine/parser/`
   (tokenizer.ts, token.ts, command.ts, code.ts, token-type-refiner.ts) into the arena model.
4. **Fixture harness:** Rust test runner over the copied JSONs, wrapper handling, ratchet baseline,
   safety invariant, idempotence.
5. **Rules, in groups:** empty lines + spaces + INSET + case (port from old repo
   `packages/core/src/engine/rules/**`, already 100% on fixtures) → ALIGN_* (needs AlignTable etc.
   from upstream Java) → DDL.
6. **Config + CLI** (incl. `.cfj` import).
7. **LSP + VS Code extension;** package VSIX; usage guide. Checkpoint: user tests.
8. **lineWidth reflow** (gofmt-style), default 120. Done.
9. **Benchmarks** vs old TS engine numbers in old repo NEXT-STEPS §0b. Done, see `benchmarks.md`.

## Verification
- `cargo test` — fixture suite (target: 100% of non-skipped keep-scope cases), safety invariant
  and idempotence over all fixtures + `cleanup-sample.abap`.
- `abapfmt --check` on sample files; diff vs upstream abap-cleaner output with the same profile.
- VS Code: install VSIX, set `editor.defaultFormatter`, run Format Document, Format Selection,
  format-on-save; confirm the keyword-collision probes (`READ TABLE lines INTO line INDEX index.`).
- Benchmark 1k/8k/32k lines vs §0b table.

## First action after approval
Only phase 1 + phase 2 spike are executed next; old repo untouched (optionally a short note in its
NEXT-STEPS.md pointing to abapfmt — ask first).
