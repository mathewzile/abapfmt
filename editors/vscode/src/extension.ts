import * as fs from 'fs';
import * as path from 'path';
import { ExtensionContext, window, workspace } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

/** The bundled build for this platform: `bin/<platform>-<arch>/abapfmt`. */
function bundledCommand(context: ExtensionContext): string | undefined {
  const exe = process.platform === 'win32' ? 'abapfmt.exe' : 'abapfmt';
  const bundled = context.asAbsolutePath(path.join('bin', `${process.platform}-${process.arch}`, exe));
  if (!fs.existsSync(bundled)) {
    return undefined;
  }
  if (process.platform !== 'win32') {
    try {
      // a VSIX built from downloaded artifacts may have lost the executable bit
      fs.chmodSync(bundled, 0o755);
    } catch {
      // read-only install location: assume the bit is set
    }
  }
  return bundled;
}

function serverCommand(context: ExtensionContext): string | undefined {
  const configured = workspace.getConfiguration('abapfmt').get<string>('path')?.trim();
  return configured ? configured : bundledCommand(context);
}

export async function activate(context: ExtensionContext): Promise<void> {
  const command = serverCommand(context);
  if (!command) {
    void window.showErrorMessage(
      `abapfmt: no build for ${process.platform}-${process.arch} is bundled. Set "abapfmt.path" to an abapfmt executable.`,
    );
    return;
  }

  const serverOptions: ServerOptions = { command, args: ['lsp'] };
  // no scheme: local files as well as documents of remote file systems (e.g. ABAP systems via ADT)
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: 'abap' }, { language: 'abap-cds' }],
    initializationOptions: {
      configPath: workspace.getConfiguration('abapfmt').get<string>('configPath') ?? '',
    },
  };
  client = new LanguageClient('abapfmt', 'abapfmt', serverOptions, clientOptions);
  context.subscriptions.push({ dispose: () => void client?.stop() });
  await client.start();
}

export async function deactivate(): Promise<void> {
  await client?.stop();
  client = undefined;
}
