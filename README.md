# abapfmt

A fast, safe formatter for ABAP and ABAP CDS (DDL/DCL), written in Rust. It ships as a command-line
tool, a language server and a VS Code extension.

abapfmt changes **whitespace, line layout and letter case only**. It never rewrites, removes or
reorders code. Every result is checked against the original token by token, and a result that
fails the check is never written.

Its formatting rules and options come from [ABAP cleaner](https://github.com/SAP/abap-cleaner)
(SAP, Apache-2.0). Existing ABAP cleaner profiles (`.cfj`) work as they are. See [NOTICE](NOTICE).

```abap
" before                                       " after
class lcl_demo implementation.                 CLASS lcl_demo IMPLEMENTATION.
method run.                                      METHOD run.
data lv_count type i .                             DATA lv_count TYPE i.
data lv_name type string.                          DATA lv_name  TYPE string.
lv_count = 1.
lv_name = `x`.                                     lv_count = 1.
if lv_count  is  not  initial.                     lv_name = `x`.
cl_demo_output=>display( exporting               IF lv_count IS NOT INITIAL.
  data = lv_name name = `Name` ).                    cl_demo_output=>display( EXPORTING data = lv_name
endif.                                                                                  name = `Name` ).
endmethod.                                         ENDIF.
endclass.                                        ENDMETHOD.
                                               ENDCLASS.
```

## Features

- **46 formatting rules** from ABAP cleaner: empty lines, spaces, one command per line,
  indentation (`INSET`), upper/lower case, all 17 `ALIGN_*` rules, and the CDS rules
  (`DDL_POSITION_*`, `DDL_ALIGN_*`, `DDL_SPACES_*`, `DDL_EMPTY_LINES_*`, `DDL_ANNO_LAYOUT`).
- **Line width**: ABAP statements longer than `lineWidth` (default 120) get line breaks.
- **Safe**: a token check runs on every result, and the formatter never writes a result that
  changes code. If a rule fails the check, only that rule is skipped, and it is named in a
  warning.
- **Accurate keyword detection**: abapfmt tells keywords from identifiers using abaplint's
  statement grammar, so `READ TABLE lines INTO line INDEX index.` comes out right.
- **Fast**: about 30,000 lines per second per core, and the CLI formats files on all cores (see
  [docs/benchmarks.md](docs/benchmarks.md)).
- **Editor support**: Format Document, Format Selection and format on save in VS Code. This
  includes `formatOnSaveMode: "modifications"` and ABAP systems opened through ADT.

## Installation

Download from the [GitHub releases](https://github.com/mathewzile/abapfmt/releases):

- **VS Code extension (all platforms).** `abapfmt-<version>.vsix`. Install it with
  *Extensions: Install from VSIX…* in VS Code. It includes the `abapfmt` executable for Windows
  x64, Linux x64, macOS Intel and macOS Apple Silicon.
- **Command line.** `abapfmt-<version>-<platform>.zip` (Windows) or `.tar.gz`. Unpack the
  `abapfmt` executable to a folder on your `PATH`.

**From source.** This needs the stable Rust toolchain (edition 2024):

```sh
cargo build --release -p abapfmt_cli      # -> target/release/abapfmt(.exe)
```

## Quick start

### Command line

```sh
abapfmt src/zcl_example.abap      # print the formatted file
abapfmt --check src/              # list unformatted files; exit 1 if there are any (CI)
abapfmt --write src/              # format files in place
abapfmt --stdin < in.abap         # format stdin
abapfmt --print-config > abapfmt.json   # every rule and option, with defaults
```

When you pass a directory, abapfmt formats every ABAP (`.abap`) and CDS (`.acds`, `.asddls`,
`.asdcls`, `.ddls`, `.dcls`, `.ddl`, `.dcl`) file in it. It skips hidden folders, `node_modules`
and `target`.

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | `--check` found files that are not formatted |
| 2 | Error: a file that cannot be read or tokenized (left unchanged), or a file for which a rule had to be skipped (the safe result is still used) |

### VS Code

Install the extension, then make abapfmt the formatter in `settings.json`:

```json
"[abap]":     { "editor.defaultFormatter": "mysug.abapfmt", "editor.formatOnSave": true },
"[abap-cds]": { "editor.defaultFormatter": "mysug.abapfmt" }
```

See [editors/vscode/README.md](editors/vscode/README.md) for the settings and troubleshooting.

## Configuration

abapfmt searches for its configuration starting in the file's directory and then in each parent
directory, nearest first. In each directory it checks `abapfmt.json`, then `.vscode/abapfmt.json`.
Without a configuration, every option has ABAP cleaner's default.

```json
{
  "lineWidth": 120,
  "tabularLineWidth": 160,
  "rules": {
    "ALIGN_PARAMETERS": { "MaxLineLength": 100 },
    "DDL_ALIGN_FIELD_LISTS": { "NameListPos": "LINE_START_PLUS_4" },
    "INSET": false
  }
}
```

- Rule ids and option names are ABAP cleaner's. Enum options are written by name. `false`, or
  `{ "enabled": false }`, switches a rule off.
- `lineWidth` is the maximum line width for ABAP; `"off"` switches line breaking off.
  `tabularLineWidth` is the allowed length of table rows in tabular style.
- An ABAP cleaner profile works directly: `abapfmt --config team.cfj src/`. To convert one, run
  `abapfmt --print-config --config team.cfj > abapfmt.json`.
- `--config FILE` and `--no-config` override the search.

The full reference, including configuration lookup in the editor, is in
[docs/usage.md](docs/usage.md).

## How it works

```
source ─► lexer ─► grammar (abaplint, generated) ─► syntax ─► 46 rules ─► reflow ─► token check ─► result
          lossless   KEYWORD vs IDENTIFIER          statements,  whitespace   lineWidth   same tokens,
          tokens                                    blocks,      and case               else the failing rule is skipped
                                                    brackets     only
```

| Crate | Role |
|---|---|
| `abapfmt_lexer` | Lossless tokenizer, ported from ABAP cleaner's `Tokenizer` |
| `abapfmt_grammar` | abaplint's statement grammar, generated into Rust tables, plus a matcher |
| `abapfmt_syntax` | Statements, chains, block nesting, brackets, and the CDS parser |
| `abapfmt_formatter` | The rules, the line-width reflow and the token check (`format_safe` skips a rule that fails it) |
| `abapfmt_config` | `abapfmt.json` and `.cfj` loading, configuration lookup |
| `abapfmt_cli` | The `abapfmt` binary, including the `abapfmt lsp` subcommand |
| `abapfmt_lsp` | Language server for document and range formatting over stdio |
| `editors/vscode` | Thin VS Code client that starts the bundled `abapfmt lsp` |

[docs/architecture.md](docs/architecture.md) covers the design, the reasoning behind it and its
trade-offs.

## Development

```sh
cargo test --workspace            # fixtures, safety, idempotence, CLI and LSP tests
cargo run --release -p abapfmt_formatter --example bench   # throughput
```

- **Fixtures.** `fixtures/rules/**` holds 1,140 test cases converted from ABAP cleaner v1.29.0's
  JUnit tests. 1,124 pass. 13 are skipped, as they are upstream, and 3 are out of scope because
  they rewrite comments. `fixtures/baseline.json` is a ratchet: a case that once passed must keep
  passing. Run `UPDATE_BASELINE=1 cargo test` to record new passes.
- **Invariants tested on every fixture.** The output has the same tokens as the input, and
  formatting the output again changes nothing.
- **Grammar.** `crates/abapfmt_grammar/src/tables.rs` is generated from `@abaplint/core` by
  `tools/gen-grammar/gen.cjs`. Do not edit it by hand. The weekly `refresh-grammar` workflow opens
  a pull request when a newer abaplint is available. To regenerate by hand, see
  [docs/grammar.md](docs/grammar.md).
- **CI.** `.github/workflows/build.yml` tests on Windows, Linux and macOS, then packages one VSIX
  for all platforms.
- **Releases.** Set the same version in `Cargo.toml` and `editors/vscode/package.json`, then push
  the tag `v<version>`. CI publishes a GitHub release with the VSIX and the CLI archives.
- Every crate declares `#![forbid(unsafe_code)]`.

## Known limitations

- ABAP cleaner rules that rewrite code (chain removal, declaration changes, comment merging,
  `DDL_ANNO_NESTING`, …) are not implemented and are rejected in `abapfmt.json`.
- Chained statements (`CALL METHOD: …`) are not aligned. CDS sources, chains and comments are not
  reflowed for `lineWidth`.
- All ADT systems in a workspace share one configuration.
- Only the four bundled platforms are supported out of the box. On other platforms, set
  `abapfmt.path` to your own build.

The complete list is in [docs/usage.md](docs/usage.md#known-limitations).

## License

Apache-2.0. Rule options and test fixtures are derived from SAP's ABAP cleaner (Apache-2.0). The
generated grammar is derived from abaplint (MIT). Details are in [NOTICE](NOTICE).
