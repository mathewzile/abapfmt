# Architecture and design evaluation

This document describes how abapfmt is built, why it is built this way, and where the approach is
strong or weak. For usage, see [usage.md](usage.md). For the original plan and phases, see
[plan.md](plan.md).

## Goal and the one rule

abapfmt began as a replacement for a TypeScript port of ABAP cleaner. That port grew too complex:
about 60 of ABAP cleaner's 98 rules rewrite code. Those rules need semantic helpers (class,
variable and logical-expression analysis), depend on running in a fixed order, and needed a
line-diff adapter to turn their output into edits.

abapfmt keeps only the part of ABAP cleaner that is formatting:

> **A rule may change whitespace, line layout and letter case. It may never change a token.**

The rest of the design follows from this rule. A formatter of this kind needs no semantic model.
It is safe to run on save. And its correctness can be checked mechanically on every run.

## Pipeline

```
           ┌────────────┐  ┌─────────────────┐  ┌─────────────┐  ┌────────────────┐
 source ──►│   lexer    │─►│ grammar matcher │─►│   syntax    │─►│      Doc       │
           │ (lossless) │  │ (abaplint, gen.)│  │ (commands,  │  │ lines/spaces/  │
           └────────────┘  └─────────────────┘  │  blocks,    │  │ case per token │
                                                │  brackets)  │  └───────┬────────┘
                                                └─────────────┘          │
                         ┌──────────────── 46 rules, in ABAP cleaner order ┘
                         ▼
                   ┌──────────┐   ┌──────────────┐   ┌──────────────┐
                   │  render  │──►│ reflow       │──►│ same_tokens  │──► result (or error)
                   └──────────┘   │ (lineWidth)  │   │ token check  │
                                  └──────────────┘   └──────────────┘
```

### 1. Lexer (`abapfmt_lexer`)

The lexer is a lossless tokenizer, ported from ABAP cleaner's `Tokenizer`. Tokens are spans into
the source. The whitespace before each token is summarised as *line breaks before* and *spaces
before*. Comments, pragmas, string templates and the chain characters `:` and `,` are tokens of
their own. At this stage every word has the type `Word`; whether it is a keyword is decided later.

### 2. Grammar (`abapfmt_grammar`)

Formatting ABAP depends on telling keywords from identifiers. In
`READ TABLE lines INTO line INDEX index.`, `lines`, `line` and `index` are identifiers even though
each of them is also an ABAP keyword. Guessing from a keyword list gets cases like this wrong.

abapfmt takes the declarative statement and expression grammar of
[abaplint](https://github.com/abaplint/abaplint) and generates Rust tables from it
(`tools/gen-grammar/gen.cjs` writes `tables.rs`). The generator loads the compiled
`@abaplint/core` package and walks its combinator objects (`seq`, `alt`, `opt`, `per`, …) rather
than parsing TypeScript. `ver()` gates keep the ABAP release attached. All 317 statements and all
269 reachable expressions translate declaratively, with no hand-written overrides. A matcher
modelled on abaplint's `combi.ts` runs these tables. A word matched by `str()` is a keyword, and a
word matched by `regex()` is an identifier.

When the grammar cannot parse a statement (macros, native SQL, syntax newer than the pinned
abaplint version), its words fall back to a guess based on the grammar's own set of literal words.
Formatting does not fail. Across the fixture corpus, 99.3% of commands are matched fully, and no
fixture fails because of a fallback word.

Matching is the most expensive step. Results are therefore cached by the exact tokens of a
statement, which works well because rules never change tokens.

### 3. Syntax (`abapfmt_syntax`)

This crate groups tokens into commands (statements and chains) and records bracket pairs, block
nesting (`METHOD`/`IF`/`LOOP`/class sections) and operator types. CDS sources (DDL/DCL) have their
own parser (`ddl.rs`). `is_ddl_source` picks the parser for a file. DDL rules run only on CDS and
ABAP rules only on ABAP.

### 4. The `Doc` and the rules (`abapfmt_formatter`)

A `Doc` is the parsed source plus three editable arrays, one entry per token: line breaks before,
spaces before, and an optional replacement text (for case and comment-sign changes). Rules edit
these arrays and nothing else. When a `Doc` is rendered, every token whose layout no rule touched
keeps its original whitespace, so untouched code stays byte-identical.

The rules run in ABAP cleaner's `RuleID` order (`rules/mod.rs`). Each rule receives its options
merged over ABAP cleaner's defaults (`defaults.json`). Some rules also read options of other rules,
as they do upstream (`CROSS_RULE_SETTINGS`). The text is parsed again only after a rule has changed
it. A rule that moves tokens (for example a period in front of a trailing comment) can set
`doc.rerun` to run once more on the parsed result.

Alignment rules share a table layout engine (`align.rs`), ported from ABAP cleaner's `AlignTable`.

### 5. Reflow (`reflow.rs`)

When `lineWidth` is set, ABAP statements with lines that are too long get line breaks after all
rules have run. The breaks fall at token boundaries, preferably at the shallowest bracket depth,
in the style of gofmt. Lines that already fit are never touched. After a reflow the rules run
again, and the reflow repeats until the output is stable, so that formatting the result again
changes nothing. Rows of tables in tabular style may be up to `tabularLineWidth` long. Chains,
comments and CDS sources are not reflowed.

### 6. Token check (`same_tokens`)

`format_checked` compares the result with the input before returning it:

- Code tokens must match in order. Keywords and identifiers are compared case-insensitively, and
  literals exactly.
- Comments are compared as a separate stream, with whitespace inside them ignored, because rules
  may re-indent commented-out code and move a period in front of a comment.
- Pragmas are compared as a separate stream, ignoring case.
- Two known exceptions are normalised before the comparison: `ALIGN_PERFORM` moving `IF FOUND`,
  and CDS annotation paths being joined (`@Anno. sub` → `@Anno.sub`).

The CLI and the language server call `format_safe`. If the check of the whole result fails, it
runs the rules again with a token check after every rule that changed the text. It switches off
each rule that fails, and then the `lineWidth` reflow if the result still fails, and formats again
with the rest. The safe result is used, and the skipped rules are reported: as warnings in the
editor's output channel, and on stderr with exit code 2 in the CLI, so CI notices. Only a source
that cannot be tokenized is rejected as a whole. A bug in a rule can therefore cost that rule's
edits, but never corrupt code.

## Configuration (`abapfmt_config`)

`abapfmt.json` maps ABAP cleaner rule ids to options (or to `false`), and sets `lineWidth` and
`tabularLineWidth`. `.cfj` profiles are imported directly: enum ordinals are converted, and rules
abapfmt does not implement are ignored with a warning. In `abapfmt.json` the same unimplemented
rules are errors, so a configuration cannot silently promise something abapfmt does not do.

The search goes from the file's directory up through its parents. In each directory it checks
`abapfmt.json`, then `.vscode/abapfmt.json`. The language server resolves a configuration in this
order:

1. `abapfmt.configPath`, if set (relative to the first workspace folder).
2. The nearest configuration of a local file.
3. The configuration of a workspace folder. This applies to remote documents such as ADT.
4. The defaults.

The configuration is read again on every format request, so edits to `abapfmt.json` take effect
without a restart.

## Editor integration

`abapfmt lsp` is a subcommand of the same binary, so the CLI and the editor cannot drift apart.
The server (`lsp-server`, stdio) advertises only full text sync, `documentFormatting` and
`documentRangeFormatting`.

- **Document formatting** formats the whole text and returns line-based edits, computed with a
  line diff (`similar`).
- **Range formatting** also formats the whole document, because alignment and indentation depend
  on the surrounding code. It then returns only the edits that touch the selected lines. VS Code's
  `formatOnSaveMode: "modifications"` uses the same path.
- Each request logs the configuration it used, and any skipped rule, to the client's **abapfmt**
  output channel.

The VS Code extension (`editors/vscode/src/extension.ts`, about 100 lines) contributes the `abap`
and `abap-cds` languages and starts `bin/<platform>-<arch>/abapfmt lsp`, or the executable named by
`abapfmt.path`. It passes `abapfmt.configPath` as an initialization option, and starts a new server
when either setting changes. Its document selector has no scheme, so documents of remote file
systems are formatted too.

## Command line

The CLI resolves the configuration of every file first, in order, so that each one is loaded and
warned about once. It then formats the files on all cores (`std::thread::scope`, one worker per
core), and reports results in the order of the files, so output is deterministic. The parser's
statement cache is shared behind a lock.

## Release and maintenance

- `.github/workflows/build.yml` tests on Windows, Linux and macOS, builds the four platform
  binaries and packs them into one VSIX. On a `v*` tag it also publishes a GitHub release with the
  VSIX and an archive of the CLI per platform. `scripts/check-version.mjs` makes sure that
  `Cargo.toml`, `package.json` and the tag name the same version.
- `.github/workflows/refresh-grammar.yml` runs every week. When npm has a newer `@abaplint/core`,
  it regenerates `tables.rs`, runs the tests and opens a pull request with the result (a draft if
  the tests fail). The repository must allow GitHub Actions to create pull requests.
- The ABAP cleaner side stays manual. `tools/import-java-fixtures.mjs` imports the tests of a new
  upstream version, but rule changes have to be ported by hand.

## Testing strategy

| Layer | Test |
|---|---|
| Rules | 1,140 ABAP cleaner JUnit cases (`fixtures/rules`), with a ratchet baseline (1,124 passing) |
| Safety | Every rule's output on every fixture passes `same_tokens` |
| Idempotence | Every rule and the full pipeline: `fmt(fmt(x)) == fmt(x)` (`stability.rs`) |
| Grammar | 33 keyword-collision probes and whole-file matching (`abapfmt_grammar/tests`) |
| Syntax | Corpus gate over all fixture snippets and samples (`corpus.rs`) |
| CLI / LSP | Exit codes and options; protocol round-trips and configuration lookup |

## Evaluation

### What works well

- **Safety is checked, not assumed.** The token check makes the riskiest failure (changing code)
  impossible to ship silently. This is what makes format on save trustworthy for ABAP, where a
  changed token can change behaviour without a syntax error. A failing rule is contained: only its
  edits are lost, and it is named in the output.
- **Accurate keyword detection without a hand-written parser.** Generating the grammar from
  abaplint gives full statement coverage, and supporting new syntax means regenerating from a newer
  version. The fallback ensures an unknown statement degrades to a guess rather than an error.
- **Compatibility is measurable.** Using ABAP cleaner's rule ids, options, defaults and test suite
  means compatibility can be counted in passing cases rather than argued about. Teams can keep
  their existing profiles.
- **Minimal diffs.** Untouched tokens keep their bytes, and user line breaks are preserved (ABAP
  cleaner's model). Changes are confined to what a rule actually changed, which matters in shared
  repositories and in transports.
- **One binary, thin client.** The CLI, CI checks and the editor all run the same code. The
  extension has nothing to maintain beyond start-up.
- **Performance.** At about 30k lines per second per core (4–6× faster than the TypeScript port;
  1.2 s vs 7.6 s for 32k lines), format on save is instant for any realistic ABAP object. The CLI
  uses all cores (0.35 s for 200 files with 32k lines on 12 cores).

### Trade-offs and risks

- **Repeated parsing (deferred).** The text is parsed again after each rule that changes it (13 of
  46 on the sample). With the warm statement cache a re-parse takes about 1 ms for 800 lines, so
  rules that merge their changes into a shared `Doc` would save little and risk subtle
  differences. It is not worth it at current speeds.
- **Pinned dependencies.** The grammar is pinned to one abaplint version and the rule semantics to
  ABAP cleaner v1.29.0. Newer ABAP statements fall back to guessing until the grammar is
  regenerated. The weekly refresh workflow proposes that as a pull request. Upstream rule fixes
  still have to be ported by hand.
- **Reflow is heuristic (deferred).** Break points are chosen by bracket depth and operator rules,
  not by a pretty-printing algorithm with a cost model. Results are stable and idempotent, but not
  always the break a person would pick. Chains, comments and CDS are not reflowed at all. A
  redesign needs example files where the current breaks are bad.
- **Range formatting needs a tokenizable document.** Range formatting returns only the selected
  lines' edits, but it formats the whole document. A failing rule is skipped, but a document that
  cannot be tokenized (for example an unclosed `lv_c(20` while typing) gets no edits.
- **GitHub releases only.** The extension is not on the Marketplace or Open VSX. That needs a
  publisher account and a token stored as a repository secret.

### Possible next steps

1. Publish to the Marketplace and Open VSX from the release job.
2. A cost-based reflow, driven by real examples of bad breaks.
3. Alignment of chained statements.
