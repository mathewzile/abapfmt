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

abapfmt looks for the nearest `abapfmt.json` in the file's directory and its parents
(`--config <file>` and `--no-config` override this).

```json
{
  "rules": {
    "ALIGN_PARAMETERS": { "MaxLineLength": 100 },
    "DDL_ALIGN_FIELD_LISTS": { "NameListPos": "LINE_START_PLUS_4" },
    "INSET": false
  }
}
```

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
