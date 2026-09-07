import * as vscode from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';

/** The `glue/syntaxTree` response. Mirrors `SyntaxNodeInfo` in the server. */
export interface SyntaxNode {
  kind: string;
  isToken: boolean;
  range: { start: { line: number; character: number }; end: { line: number; character: number } };
  text: string | null;
  children: SyntaxNode[];
}

export function toRange(node: SyntaxNode): vscode.Range {
  return new vscode.Range(
    node.range.start.line,
    node.range.start.character,
    node.range.end.line,
    node.range.end.character
  );
}

export async function fetchTree(
  client: LanguageClient,
  document: vscode.TextDocument
): Promise<SyntaxNode | null> {
  return client.sendRequest<SyntaxNode | null>('glue/syntaxTree', {
    textDocument: { uri: document.uri.toString() },
  });
}

/** The tree panel, and the two-way sync between it and the cursor. */
export class SyntaxTreeProvider implements vscode.TreeDataProvider<SyntaxNode> {
  private changed = new vscode.EventEmitter<SyntaxNode | undefined>();
  readonly onDidChangeTreeData = this.changed.event;

  private root: SyntaxNode | null = null;
  private parents = new Map<SyntaxNode, SyntaxNode>();

  setTree(root: SyntaxNode | null): void {
    this.root = root;
    this.parents.clear();
    if (root) this.index(root);
    this.changed.fire(undefined);
  }

  private index(node: SyntaxNode): void {
    for (const child of node.children) {
      this.parents.set(child, node);
      this.index(child);
    }
  }

  getChildren(node?: SyntaxNode): SyntaxNode[] {
    if (!node) return this.root ? [this.root] : [];
    return node.children;
  }

  // Required for `TreeView.reveal`, which is how the cursor drives the panel.
  getParent(node: SyntaxNode): SyntaxNode | undefined {
    return this.parents.get(node);
  }

  getTreeItem(node: SyntaxNode): vscode.TreeItem {
    const item = new vscode.TreeItem(
      node.kind,
      node.children.length > 0
        ? vscode.TreeItemCollapsibleState.Expanded
        : vscode.TreeItemCollapsibleState.None
    );
    const { start, end } = node.range;
    item.description = node.isToken
      ? JSON.stringify(node.text ?? '')
      : `${start.line + 1}:${start.character + 1}–${end.line + 1}:${end.character + 1}`;
    item.tooltip = `${node.kind}\n${start.line + 1}:${start.character + 1}–${end.line + 1}:${
      end.character + 1
    }`;
    item.command = {
      command: 'glue.revealNode',
      title: 'Reveal in editor',
      arguments: [node],
    };
    return item;
  }

  /** The deepest node whose range contains `position`. */
  nodeAt(position: vscode.Position): SyntaxNode | undefined {
    let found: SyntaxNode | undefined;
    const visit = (node: SyntaxNode): void => {
      if (!toRange(node).contains(position)) return;
      found = node;
      for (const child of node.children) visit(child);
    };
    if (this.root) visit(this.root);
    return found;
  }
}
