# abapfmt for VS Code

Formats ABAP and CDS (DDL/DCL) code with the abapfmt language server: Format Document, Format Selection and format on save.

Set `"editor.defaultFormatter": "abapfmt.abapfmt"` for the `abap` and `abap-cds` languages. Rules are configured in an `abapfmt.json` or `.vscode/abapfmt.json` next to your code (or in a parent folder).

The extension also works for documents of remote file systems (for example ABAP systems opened with ADT); they use the `abapfmt.json` or `.vscode/abapfmt.json` of the workspace folder, or the file named by the `abapfmt.configPath` setting. See `docs/usage.md` for the configuration and the known limitations.
