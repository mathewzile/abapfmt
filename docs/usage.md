# Using abapfmt

## Command line

```
abapfmt --check src/            # list files that are not formatted (exit 1 if any)
abapfmt --write src/            # rewrite files in place
abapfmt src/zcl_example.abap    # print the formatted code of one file
abapfmt --stdin < in.abap       # format stdin
```

ABAP (`.abap`) and CDS (`.acds`, `.asddls`, `.asdcls`, `.ddls`, `.dcls`, `.ddl`, `.dcl`) files are
found in directories and formatted on all cores; results are reported in the order of the files.

Exit codes: 0 success, 1 `--check` found unformatted files, 2 error. An error is a file that
cannot be read or tokenized (it is left untouched), or a file for which a rule had to be skipped.

Every result is checked: apart from whitespace, line layout and letter case, it must have the same
tokens as the input. If a rule's result fails that check (a bug in abapfmt), the rule is switched
off for that file, the other rules still apply, and a warning names the rule:

```
warning: src/zcl_x.abap: rule ALIGN_PARAMETERS skipped, its result would have changed the code (...)
```

With `--write` the safe result is written; the exit code is still 2, so that CI notices.

## Configuration

abapfmt looks for the configuration in the directory of the file and in each parent directory,
nearest first. In every directory it checks `abapfmt.json` and `.vscode/abapfmt.json`, in this order. This covers ABAP files that are committed to a repository.
`--config <file>` and `--no-config` override the search.

In the editor, documents of remote file systems (for example ABAP systems opened through ADT) have
no directory to search from; for them, and for local files without a configuration, the server uses
the configuration found in the workspace folders (same locations, e.g. `.vscode/abapfmt.json` of
the workspace). All systems of a workspace therefore share one configuration. The setting
`abapfmt.configPath` names a file explicitly (anywhere, for example `config/abapfmt.json`) and takes precedence.
Each time a document is formatted, the server logs which configuration it used (or that it fell back
to the defaults) to the **abapfmt** output channel (View > Output).

```json
{
  "rules": {
    "ALIGN_PARAMETERS": { "MaxLineLength": 100 },
    "DDL_ALIGN_FIELD_LISTS": { "NameListPos": "LINE_START_PLUS_4" },
    "INSET": false
  },
  "lineWidth": 120,
  "tabularLineWidth": 160
}
```

`lineWidth` (default 120, as recommended by Clean ABAP) is the maximum line width for ABAP: after
all rules have run, statements with longer lines get line breaks at token boundaries, preferably
where the nesting is shallowest. Lines that fit are left alone. `"lineWidth": "off"` switches this
off, so that only the line length options of the individual rules apply. CDS sources are not
reflowed; their rules have their own `MaxLineLength` options. Chains and comments are not broken.
A line that cannot be broken at a token boundary stays as it is, even if it exceeds the width.

`tabularLineWidth` (default 160, as in abap-cleaner) is the maximum length of the rows of a table in
tabular style, where some overlength is tolerated because the table stays readable. It is used as
`MaxLineLengthForSingleLine` of `ALIGN_PARAMETERS` (a value set there wins), and the reflow leaves
lines that start with a table row within this length alone.

`abapfmt --print-config` prints the complete configuration: every rule with all of its options and
their defaults, ready to be saved as `abapfmt.json` and edited. With `--config FILE` it prints the
effective configuration of that file instead, which also converts an abap-cleaner profile to
`abapfmt.json`:

```
abapfmt --print-config > abapfmt.json
abapfmt --print-config --config essential.cfj > abapfmt.json
```

Options that are not set in the configuration have abap-cleaner's default. Rule ids and option names are those of abap-cleaner; enum options are written by name. `false`
(or `"enabled": false`) switches a rule off. Rules that abapfmt does not implement (those that
rewrite code) are rejected in `abapfmt.json`.

An abap-cleaner profile (`.cfj`) can be used directly: `abapfmt --config profile.cfj ...`. Rules
it activates that abapfmt does not implement are ignored with a warning.

## VS Code

**One extension for all platforms.** Download `abapfmt-<version>.vsix` from the
[GitHub releases](https://github.com/mathewzile/abapfmt/releases). It contains the builds for
Windows (x64), Linux (x64) and macOS (Intel and Apple Silicon), and the extension starts the one
for the platform it runs on. The releases also have the CLI on its own for each platform.

**Releasing.** The workflow `.github/workflows/build.yml` builds and tests on every platform and
packages the VSIX (also as the `abapfmt-vsix` artifact of any run). To publish a release, set the
same version in `Cargo.toml` (`[workspace.package]`) and `editors/vscode/package.json`, then push
the tag `v<version>`. The workflow fails if the versions and the tag differ.

**Local build for this platform only** (for development), from `editors/vscode`:

```
cargo build --release -p abapfmt_cli
cd editors/vscode && npm install && npm run package
```

Install the resulting `.vsix`, then set it as the formatter:

```json
"[abap]":     { "editor.defaultFormatter": "mysug.abapfmt", "editor.formatOnSave": true },
"[abap-cds]": { "editor.defaultFormatter": "mysug.abapfmt" }
```

Format Document, Format Selection and format on save (including `editor.formatOnSaveMode:
"modifications"`) all use the language server (`abapfmt lsp`). Set `abapfmt.path` to use a
different executable. Changing `abapfmt.path` or `abapfmt.configPath` restarts the server. A rule
that had to be skipped is reported as a warning in the **abapfmt** output channel; the other
rules still apply.

## Known limitations

These are deliberate choices or accepted approximations. `architecture.md` lists what could still
change and why the rest stays.

- **Rules that rewrite code are not implemented** (for example chain removal, `RemoveIncludeWarnings`,
  merging comments, `DDL_ANNO_NESTING`). abapfmt changes only whitespace, line layout and letter
  case, and never uses a result in which any other token differs.
- **Chained statements** (`CALL METHOD: ...`, `CHECK: ...`) are not aligned by `ALIGN_PARAMETERS` and
  `ALIGN_LOGICAL_EXPRESSIONS`; abap-cleaner has no tests for them either, and aligning them gives
  worse results than leaving them alone.
- **Line width:** a line that cannot be broken at a token boundary stays over `lineWidth`. Chains,
  comments and CDS sources are not reflowed, and the choice of break points is a heuristic.
- **Commented-out code in CDS select lists:** whether a comment between elements is commented-out
  code or text is decided by a simple heuristic, not by abap-cleaner's word-frequency classifier.
  It agrees with all of abap-cleaner's test cases for these rules.
- **One configuration per workspace:** documents of all remote systems (ADT) share the workspace's
  `abapfmt.json`; there are no per-system settings.
- **Speed:** about 30,000 lines per second per core; the CLI formats files on all cores (see
  `benchmarks.md`). Each rule that changes the text causes one more parse; this is not a problem at
  these speeds and is not being optimised further.
- **Platforms:** the VSIX and the CLI archives of the releases support Windows x64, Linux x64,
  macOS Intel and macOS Apple Silicon. Other platforms (for example Linux or Windows on ARM) need `abapfmt.path` to point at
  a build of abapfmt for that platform.
