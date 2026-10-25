import * as fs from 'fs';
import * as path from 'path';
import { ExtensionContext, window, workspace } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

function serverCommand(context: ExtensionContext): string | undefined {
  const configured = workspace.getConfiguration('abapfmt').get<string>('path')?.trim();
  if (configured) {
    return configured;
  }
  const exe = process.platform === 'win32' ? 'abapfmt.exe' : 'abapfmt';
  const bundled = context.asAbsolutePath(path.join('bin', exe));
  return fs.existsSync(bundled) ? bundled : undefined;
}

export async function activate(context: ExtensionContext): Promise<void> {
  const command = serverCommand(context);
  if (!command) {
    void window.showErrorMessage(
      'abapfmt: no executable found. Set "abapfmt.path" or install a build of the extension that bundles it.',
    );
    return;
  }

  const serverOptions: ServerOptions = { command, args: ['lsp'] };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: 'file', language: 'abap' },
      { scheme: 'file', language: 'abap-cds' },
      { scheme: 'untitled', language: 'abap' },
      { scheme: 'untitled', language: 'abap-cds' },
    ],
  };
  client = new LanguageClient('abapfmt', 'abapfmt', serverOptions, clientOptions);
  context.subscriptions.push({ dispose: () => void client?.stop() });
  await client.start();
}

export async function deactivate(): Promise<void> {
  await client?.stop();
  client = undefined;
}
