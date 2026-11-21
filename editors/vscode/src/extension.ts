import { execFile } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import { commands, ExtensionContext, OutputChannel, window, workspace } from 'vscode';
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

/** Starts the language server with the current settings; shows an error if there is no executable. */
async function startClient(context: ExtensionContext, outputChannel: OutputChannel): Promise<void> {
  const command = serverCommand(context);
  if (!command) {
    void window.showErrorMessage(
      `abapfmt: no build for ${process.platform}-${process.arch} is bundled. Set "abapfmt.path" to an abapfmt executable.`,
    );
    return;
  }
  outputChannel.appendLine(`abapfmt: starting ${command} lsp`);

  const serverOptions: ServerOptions = { command, args: ['lsp'] };
  // no scheme: local files as well as documents of remote file systems (e.g. ABAP systems via ADT)
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: 'abap' }, { language: 'abap-cds' }],
    outputChannel,
    initializationOptions: {
      configPath: workspace.getConfiguration('abapfmt').get<string>('configPath') ?? '',
    },
  };
  client = new LanguageClient('abapfmt', 'abapfmt', serverOptions, clientOptions);
  try {
    await client.start();
  } catch (e) {
    client = undefined;
    void window.showErrorMessage(`abapfmt: cannot start ${command}: ${e instanceof Error ? e.message : e}`);
  }
}

async function stopClient(): Promise<void> {
  const running = client;
  client = undefined;
  await running?.stop();
}

/** Writes the complete default configuration (`abapfmt --print-config`) as abapfmt.json and opens it. */
async function createConfig(context: ExtensionContext): Promise<void> {
  const folder = workspace.workspaceFolders?.[0];
  if (!folder || folder.uri.scheme !== 'file') {
    void window.showErrorMessage('abapfmt: open a local folder to create abapfmt.json in.');
    return;
  }
  const target = path.join(folder.uri.fsPath, 'abapfmt.json');
  if (!fs.existsSync(target)) {
    const command = serverCommand(context);
    if (!command) {
      void window.showErrorMessage('abapfmt: no executable found. Set "abapfmt.path".');
      return;
    }
    try {
      const json = await new Promise<string>((resolve, reject) =>
        execFile(command, ['--print-config'], { maxBuffer: 16 * 1024 * 1024 }, (err, stdout) =>
          err ? reject(err) : resolve(stdout),
        ),
      );
      fs.writeFileSync(target, json);
    } catch (e) {
      void window.showErrorMessage(`abapfmt: cannot create abapfmt.json: ${e instanceof Error ? e.message : e}`);
      return;
    }
  }
  await window.showTextDocument(await workspace.openTextDocument(target));
}

export async function activate(context: ExtensionContext): Promise<void> {
  context.subscriptions.push(commands.registerCommand('abapfmt.createConfig', () => createConfig(context)));

  // created up front so that it is listed even before the server has logged anything; shared by
  // every client that a settings change starts
  const outputChannel = window.createOutputChannel('abapfmt');
  context.subscriptions.push(outputChannel);
  context.subscriptions.push({ dispose: () => void stopClient() });

  // both settings are passed to the server at start-up, so a change needs a new server
  context.subscriptions.push(
    workspace.onDidChangeConfiguration(async (e) => {
      if (e.affectsConfiguration('abapfmt.path') || e.affectsConfiguration('abapfmt.configPath')) {
        outputChannel.appendLine('abapfmt: restarting (settings changed)');
        await stopClient();
        await startClient(context, outputChannel);
      }
    }),
  );

  await startClient(context, outputChannel);
}

export async function deactivate(): Promise<void> {
  await stopClient();
}
