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

/**
 * Depth as ink density.
 *
 * Colour is a nominal channel — six hues say "different", not "deeper". A
 * background tint is ordinal: more nesting, more ink, and the eye reads the
 * ordering without being told it. The server's per-token colours keep saying
 * where one token ends and the next begins, which is what hue is good at.
 *
 * The tints do not stack. Every character belongs to exactly one token — the
 * tree is lossless, so whitespace is a token too — which makes the tokens a
 * partition of the document. Each gets the one decoration for its own depth,
 * so a level's brightness is exact rather than however many translucent layers
 * happened to overlap there.
 */
export class DepthTint {
  private decorations: vscode.TextEditorDecorationType[] = [];
  private enabled = true;

  /** Beyond this many levels the tint stops darkening, or deep code goes opaque. */
  private static readonly MAX_DEPTH = 10;
  private static readonly STEP = 0.032;

  constructor() {
    for (let depth = 0; depth <= DepthTint.MAX_DEPTH; depth++) {
      const alpha = depth * DepthTint.STEP;
      this.decorations.push(
        vscode.window.createTextEditorDecorationType({
          // Ink, not colour: dark type on light themes and the reverse, so
          // "deeper" means the same thing in both.
          light: { backgroundColor: `rgba(0, 0, 0, ${alpha})` },
          dark: { backgroundColor: `rgba(255, 255, 255, ${alpha})` },
          rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
        })
      );
    }
  }

  toggle(): boolean {
    this.enabled = !this.enabled;
    return this.enabled;
  }

  apply(editor: vscode.TextEditor, root: SyntaxNode | null): void {
    const buckets: vscode.Range[][] = this.decorations.map(() => []);

    if (this.enabled && root) {
      const visit = (node: SyntaxNode, depth: number): void => {
        if (node.isToken) {
          buckets[Math.min(depth, DepthTint.MAX_DEPTH)].push(toRange(node));
          return;
        }
        for (const child of node.children) visit(child, depth + 1);
      };
      visit(root, 0);
    }

    // Every decoration is set every time, empty ones included — that is what
    // clears the previous paint.
    this.decorations.forEach((decoration, depth) =>
      editor.setDecorations(decoration, buckets[depth])
    );
  }

  dispose(): void {
    this.decorations.forEach((decoration) => decoration.dispose());
  }
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
