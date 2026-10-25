# Using abapfmt

## Command line

```
abapfmt --check src/            # list files that are not formatted (exit 1 if any)
abapfmt --write src/            # rewrite files in place
abapfmt src/zcl_example.abap    # print the formatted code of one file
abapfmt --stdin < in.abap       # format stdin
```

ABAP (`.abap`) and CDS (`.acds`, `.asddls`, `.asdcls`, `.ddls`, `.dcls`, `.ddl`, `.dcl`) files are
found in directories. Exit codes: 0 success, 1 `--check` found unformatted files, 2 error (a file
that cannot be tokenized or whose formatting would change more than whitespace, layout or letter
case is left untouched).

## Configuration

abapfmt looks for the configuration in the directory of the file and in each parent directory,
nearest first. In every directory it checks `abapfmt.json` and `.vscode/abapfmt.json`, in this order. This covers ABAP files that are committed to a repository.
`--config <file>` and `--no-config` override the search.

In the editor, documents of remote file systems (for example ABAP systems opened through ADT) have
no directory to search from; for them, and for local files without a configuration, the server uses
the configuration found in the workspace folders (same locations, e.g. `.vscode/abapfmt.json` of
the workspace). All systems of a workspace therefore share one configuration. The setting
`abapfmt.configPath` names a file explicitly (anywhere, for example `config/abapfmt.json`) and takes precedence.

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

Build the extension from `editors/vscode`:

```
cargo build --release -p abapfmt_cli
cd editors/vscode && npm install && npm run package
```

Install the resulting `.vsix`, then set it as the formatter:

```json
"[abap]":     { "editor.defaultFormatter": "abapfmt.abapfmt", "editor.formatOnSave": true },
"[abap-cds]": { "editor.defaultFormatter": "abapfmt.abapfmt" }
```

Format Document, Format Selection and format on save (including `editor.formatOnSaveMode:
"modifications"`) all use the language server (`abapfmt lsp`). Set `abapfmt.path` to use a
different executable.

## Known limitations

These are deliberate choices or accepted approximations; none of them is planned to change.

- **Rules that rewrite code are not implemented** (for example chain removal, `RemoveIncludeWarnings`,
  merging comments, `DDL_ANNO_NESTING`). abapfmt changes only whitespace, line layout and letter
  case, and refuses to write a result in which any other token differs.
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
- **Speed:** about 30,000 lines per second (see `benchmarks.md`). Each rule that changes the text
  causes one more parse; this is not a problem at these speeds and is not being optimised further.
- **The VS Code extension** is packaged for the platform it was built on only. Build it on each
  platform you need.
