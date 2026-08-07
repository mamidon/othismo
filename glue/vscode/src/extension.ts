import * as path from 'path';
import { ExtensionContext, workspace, window, commands } from 'vscode';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

import { DepthTint, SyntaxNode, SyntaxTreeProvider, fetchTree, toRange } from './syntaxTree';

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
    return;
  }

  registerTreeViews(context, client);
}

/**
 * The two views of the tree: the tint over the source, and the panel beside it.
 *
 * Both are fed by one `glue/syntaxTree` request per refresh, so they can never
 * disagree about what the parse is.
 */
function registerTreeViews(context: ExtensionContext, client: LanguageClient): void {
  const tint = new DepthTint();
  const provider = new SyntaxTreeProvider();
  const view = window.createTreeView('glueSyntaxTree', {
    treeDataProvider: provider,
    showCollapseAll: true,
  });

  context.subscriptions.push(tint, view);

  const isGlue = (document?: vscode.TextDocument): boolean => document?.languageId === 'glue';

  let refreshing: Promise<void> | undefined;
  const refresh = async (): Promise<void> => {
    const editor = window.activeTextEditor;
    if (!editor || !isGlue(editor.document)) {
      provider.setTree(null);
      return;
    }
    let tree: SyntaxNode | null = null;
    try {
      tree = await fetchTree(client, editor.document);
    } catch {
      // The server is restarting, or the document closed mid-flight. The
      // views keep whatever they had rather than blanking.
      return;
    }
    // The editor may have moved on while the request was in flight.
    if (window.activeTextEditor !== editor) return;
    provider.setTree(tree);
    tint.apply(editor, tree);
  };

  // Coalesce: a burst of keystrokes should cost one request, not one each.
  let pending: NodeJS.Timeout | undefined;
  const scheduleRefresh = (): void => {
    if (pending) clearTimeout(pending);
    pending = setTimeout(() => {
      pending = undefined;
      refreshing = refresh();
    }, 120);
  };

  context.subscriptions.push(
    workspace.onDidChangeTextDocument((event) => {
      if (isGlue(event.document)) scheduleRefresh();
    }),
    workspace.onDidOpenTextDocument((document) => {
      if (isGlue(document)) scheduleRefresh();
    }),
    window.onDidChangeActiveTextEditor(() => scheduleRefresh()),

    // Cursor moves highlight the innermost node containing it, so the panel
    // answers "where am I in the tree" without being asked.
    window.onDidChangeTextEditorSelection(async (event) => {
      if (!isGlue(event.textEditor.document) || !view.visible) return;
      await refreshing;
      const node = provider.nodeAt(event.selections[0].active);
      if (node) await view.reveal(node, { select: true, focus: false });
    }),

    commands.registerCommand('glue.revealNode', (node: SyntaxNode) => {
      const editor = window.activeTextEditor;
      if (!editor || !isGlue(editor.document)) return;
      const range = toRange(node);
      editor.selection = new vscode.Selection(range.start, range.end);
      editor.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
    }),

    commands.registerCommand('glue.toggleDepthTint', () => {
      const enabled = tint.toggle();
      window.setStatusBarMessage(`Glue: depth tint ${enabled ? 'on' : 'off'}`, 2000);
      refreshing = refresh();
    })
  );

  refreshing = refresh();
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
