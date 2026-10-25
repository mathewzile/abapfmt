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
`abapfmt.configPath` names a file explicitly (anywhere, for example `.github/abapfmt.json`) and takes precedence.

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

Rule ids and option names are those of abap-cleaner; enum options are written by name. `false`
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
