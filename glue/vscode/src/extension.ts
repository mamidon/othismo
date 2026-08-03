import * as path from 'path';
import { ExtensionContext, workspace, window } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  const configured = workspace.getConfiguration('glue').get<string>('server.path');

  // Default to the debug build in this checkout: glue/vscode -> repo root.
  const command =
    configured && configured.length > 0
      ? configured
      : context.asAbsolutePath(path.join('..', '..', 'target', 'debug', 'lsp'));

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'glue' }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher('**/*.glue'),
    },
  };

  client = new LanguageClient('glue', 'Glue Language Server', serverOptions, clientOptions);

  try {
    await client.start();
  } catch (err) {
    window.showErrorMessage(
      `Glue: could not start the language server at ${command}. ` +
        `Build it with \`just lsp\`, or set glue.server.path. (${err})`
    );
  }
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
