//! Glue language server.
//!
//! Deliberately thin: it owns the protocol, the document store, and the
//! mapping between byte offsets and LSP positions. Everything about the
//! *language* lives in `parser` and `tokenizer`, so the same front end can
//! serve the compiler and the interpreter (design goal §both-modes).

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod line_index;
mod semantic;
mod syntax_tree;

use line_index::LineIndex;
use syntax_tree::{SyntaxNodeInfo, SyntaxTreeParams};

struct Backend {
    client: Client,
    /// Full text of every open document. No incremental sync: we take the whole
    /// buffer on every keystroke and reparse it. Files are small; revisit only
    /// when there's evidence this matters.
    documents: Mutex<HashMap<Url, String>>,
    /// Negotiated during `initialize`. UTF-8 is what we ask for — spans are
    /// byte offsets into UTF-8 source (§lexical), so anything else means
    /// converting on every diagnostic.
    encoding: Mutex<PositionEncodingKind>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Backend {
            client,
            documents: Mutex::new(HashMap::new()),
            encoding: Mutex::new(PositionEncodingKind::UTF16),
        }
    }

    /// Reparse and publish. Called on open and on every change.
    async fn refresh(&self, uri: Url, text: String) {
        let diagnostics = self.diagnose(&text);
        self.documents.lock().unwrap().insert(uri.clone(), text);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    /// Everything wrong with `text`, lexical and grammatical.
    ///
    /// Both lists, always: the tokenizer and the parser are each total, so a
    /// file that lexes badly still parses and a file that parses badly still
    /// produces a tree. Reporting only the first failure would hide the rest
    /// for no gain.
    fn diagnose(&self, text: &str) -> Vec<Diagnostic> {
        let index = LineIndex::new(text);
        let lexed = tokenizer::tokenize(text);
        let parsed = parser::parse(text);

        let lexical = lexed.diagnostics.iter().map(|diagnostic| {
            (
                diagnostic.span,
                diagnostic.message(),
                diagnostic.severity(),
                "glue (lex)",
            )
        });
        let grammatical = parsed.diagnostics.iter().map(|diagnostic| {
            (
                diagnostic.span,
                diagnostic.message(),
                diagnostic.severity(),
                "glue",
            )
        });

        lexical
            .chain(grammatical)
            .map(|(span, message, severity, source)| Diagnostic {
                range: range_of(span, &index),
                severity: Some(match severity {
                    tokenizer::Severity::Error => DiagnosticSeverity::ERROR,
                    tokenizer::Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                source: Some(source.to_string()),
                message: message.to_string(),
                ..Diagnostic::default()
            })
            .collect()
    }
}

impl Backend {
    /// `glue/syntaxTree`. Not an LSP method — see [`syntax_tree`] for why it
    /// exists and what the extension does with it.
    async fn syntax_tree(&self, params: SyntaxTreeParams) -> Result<Option<SyntaxNodeInfo>> {
        let Some(text) = self.text_of(&params.text_document.uri) else {
            return Ok(None);
        };
        let index = LineIndex::new(&text);
        let parsed = parser::parse(&text);
        Ok(Some(syntax_tree::describe(&parsed.tree, &text, &index)))
    }

    fn text_of(&self, uri: &Url) -> Option<String> {
        self.documents.lock().unwrap().get(uri).cloned()
    }
}

fn range_of(span: tokenizer::Span, index: &LineIndex) -> Range {
    let (start_line, start_column) = index.line_col(span.start as usize);
    let (end_line, end_column) = index.line_col(span.end as usize);
    Range {
        start: Position::new(start_line, start_column),
        end: Position::new(end_line, end_column),
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Ask for UTF-8 if the client offers it, so a byte span is a column
        // directly. VS Code has supported this since LSP 3.17; if a client
        // doesn't, we stay on UTF-16 and owe a conversion in `line_index`.
        let client_supports_utf8 = params
            .capabilities
            .general
            .as_ref()
            .and_then(|general| general.position_encodings.as_ref())
            .is_some_and(|encodings| encodings.contains(&PositionEncodingKind::UTF8));

        let encoding = if client_supports_utf8 {
            PositionEncodingKind::UTF8
        } else {
            PositionEncodingKind::UTF16
        };
        *self.encoding.lock().unwrap() = encoding.clone();

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "glue-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(encoding),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: semantic::token_types(),
                                token_modifiers: semantic::token_modifiers(),
                            },
                            // Whole document every time, to match FULL sync.
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..SemanticTokensOptions::default()
                        },
                    ),
                ),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let encoding = self.encoding.lock().unwrap().clone();
        self.client
            .log_message(
                MessageType::INFO,
                format!("glue-lsp ready (position encoding: {})", encoding.as_str()),
            )
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.refresh(doc.uri, doc.text).await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        // FULL sync: the last content change carries the entire buffer.
        if let Some(change) = params.content_changes.pop() {
            self.refresh(params.text_document.uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.lock().unwrap().remove(&uri);
        // Clear anything we published for a file nobody is looking at.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some(text) = self.text_of(&params.text_document.uri) else {
            return Ok(None);
        };

        let index = LineIndex::new(&text);
        let parsed = parser::parse(&text);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: semantic::tokens(&parsed.tree, &index),
        })))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::build(Backend::new)
        .custom_method("glue/syntaxTree", Backend::syntax_tree)
        .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}
