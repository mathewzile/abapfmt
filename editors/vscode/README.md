# abapfmt for VS Code

Formats ABAP and ABAP CDS (DDL/DCL) code with [abapfmt](https://github.com/mathewzile/abapfmt), a
Rust formatter that uses the rules and options of SAP's ABAP cleaner.

abapfmt changes **only whitespace, line layout and letter case**. It never rewrites code, and every
result is checked token by token before it reaches your editor.

## Features

- **Format Document**, **Format Selection** and **format on save**, including
  `"editor.formatOnSaveMode": "modifications"`.
- 46 ABAP cleaner rules for ABAP and CDS: indentation, keyword case, empty lines, spaces, and
  alignment of declarations, assignments, parameters, `SELECT`s and CDS field lists.
- Statements longer than `lineWidth` (default 120) get line breaks.
- Uses existing ABAP cleaner profiles (`.cfj`) directly.
- Works with local files and with ABAP systems opened through ADT or other remote file systems.
- Includes the formatter for Windows x64, Linux x64, macOS Intel and macOS Apple Silicon. Nothing
  else needs to be installed.

## Getting started

1. Download `abapfmt-<version>.vsix` from the
   [releases](https://github.com/mathewzile/abapfmt/releases) and install it with
   *Extensions: Install from VSIX…*.
2. Make abapfmt the formatter for ABAP in your user or workspace `settings.json`:

   ```json
   "[abap]": {
     "editor.defaultFormatter": "Uprizing.abapfmt",
     "editor.formatOnSave": true
   },
   "[abap-cds]": {
     "editor.defaultFormatter": "Uprizing.abapfmt"
   }
   ```

3. Open an `.abap` file and run **Format Document** (`Shift+Alt+F` on Windows and Linux,
   `Shift+Option+F` on macOS).

The extension registers these file types if no other extension does:

| Language | Extensions |
|---|---|
| ABAP (`abap`) | `.abap` |
| ABAP CDS (`abap-cds`) | `.acds`, `.asddls`, `.asdcls`, `.ddls`, `.dcls` |

## Configuration

Formatting rules go in an `abapfmt.json`. Without one, every option has ABAP cleaner's default.

```json
{
  "lineWidth": 120,
  "rules": {
    "ALIGN_PARAMETERS": { "MaxLineLength": 100 },
    "UPPER_AND_LOWER_CASE": { "ImplementationKeywordStyle": "UPPER_CASE" },
    "INSET": false
  }
}
```

Rule ids and option names are ABAP cleaner's. `false` switches a rule off. To get a file with every
rule and option, run `abapfmt --print-config > abapfmt.json` in a terminal, or convert a profile
with `abapfmt --print-config --config team.cfj > abapfmt.json`.

### Where the extension looks

For each document, the first match wins:

1. The file named by the `abapfmt.configPath` setting.
2. For local files: `abapfmt.json` or `.vscode/abapfmt.json` in the file's folder or any parent
   folder.
3. `abapfmt.json` or `.vscode/abapfmt.json` in a workspace folder. This is how documents from ADT
   and other remote systems get a configuration.
4. ABAP cleaner's defaults.

Changes to `abapfmt.json` take effect the next time you format. To see which configuration was
used, open **View → Output** and select **abapfmt**.

### Settings

| Setting | Default | Description |
|---|---|---|
| `abapfmt.path` | `""` | Path of an `abapfmt` executable to use instead of the bundled one, for example for an unsupported platform or a development build. |
| `abapfmt.configPath` | `""` | Configuration file (`abapfmt.json` or `.cfj`) to use for all documents. Relative paths resolve against the first workspace folder. |
| `abapfmt.trace.server` | `"off"` | `messages` or `verbose` logs the language-server traffic to the output channel. |

Changing `abapfmt.path` or `abapfmt.configPath` restarts the formatter; no reload is needed.

## Troubleshooting

| Symptom | What to do |
|---|---|
| *"There is no formatter for 'abap' files installed"* or another formatter runs | Set `editor.defaultFormatter` to `Uprizing.abapfmt` for `[abap]` and `[abap-cds]`. |
| *"no build for … is bundled"* | Your platform (for example ARM Linux or Windows) is not bundled. Build abapfmt (`cargo build --release -p abapfmt_cli`) and set `abapfmt.path`. |
| The output channel shows *"rule … skipped, its result would have changed the code"* | A rule produced a result that would have changed your code, so abapfmt switched that rule off for this file. The other rules still applied. Please report the snippet and the rule. |
| Formatting does nothing, and the output channel shows *"source cannot be tokenized"* | The file has an incomplete token, for example `lv_c(20` without the closing parenthesis. Fix it and format again. |
| The wrong rules are applied | Check the output channel for the configuration that was used. For ADT documents, put the configuration in the workspace folder or set `abapfmt.configPath`. |

## Limitations

- Only formatting is done. ABAP cleaner rules that rewrite code (for example removing chains or
  changing declarations) are not available, and `abapfmt.json` rejects them.
- Format Selection changes only the selected lines, but the whole file must be tokenizable.
- Chained statements (`CALL METHOD: …`) are not aligned. CDS sources and comments are not wrapped
  to `lineWidth`.
- All remote (ADT) systems in a workspace share one configuration.

## More

- [Project README](https://github.com/mathewzile/abapfmt#readme): command line and CI usage
- [Usage reference](https://github.com/mathewzile/abapfmt/blob/main/docs/usage.md)
- [Architecture](https://github.com/mathewzile/abapfmt/blob/main/docs/architecture.md)

License: Apache-2.0. Based on [ABAP cleaner](https://github.com/SAP/abap-cleaner) (SAP, Apache-2.0)
and [abaplint](https://github.com/abaplint/abaplint) (MIT).
