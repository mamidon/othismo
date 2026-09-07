//! Glue language server.
//!
//! Deliberately thin: it owns the protocol, the document store, and the
//! mapping between byte offsets and LSP positions. Everything about the
//! *language* lives in `tokenizer`, `parser`, and `ir`, so the same front end
//! serves the editor, the interpreter, and the wasm back end when it arrives
//! (design goal §both-modes). A squiggle in the editor is therefore the same
//! diagnostic the compiler gives, produced by the same code rather than by a
//! second opinion that can drift from it.

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

    /// Everything wrong with `text`. See [`diagnostics_for`].
    fn diagnose(&self, text: &str) -> Vec<Diagnostic> {
        diagnostics_for(text)
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

/// Everything wrong with `text` — lexical, grammatical, and, once those
/// are clean, everything elaboration has to say.
///
/// The first two lists come out together, always: the tokenizer and the
/// parser are each total, so a file that lexes badly still parses and a
/// file that parses badly still produces a tree. Reporting only the first
/// failure would hide the rest for no gain.
///
/// **Elaboration is different, and waits.** Its diagnostics are about a
/// program, and a half-typed one is not yet a program: an unfinished `let`
/// leaves a name unbound, and reporting that on every keystroke would bury
/// the syntax error that actually explains it. So type errors appear once
/// the file parses, which is the rule every other language server in this
/// shape follows.
fn diagnostics_for(text: &str) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    let lexed = tokenizer::tokenize(text);
    let parsed = parser::parse(text);

    let lexical = lexed.diagnostics.iter().map(|diagnostic| {
        (
            diagnostic.span,
            diagnostic.message().to_string(),
            diagnostic.severity(),
            "glue (lex)",
        )
    });
    let grammatical = parsed.diagnostics.iter().map(|diagnostic| {
        (
            diagnostic.span,
            diagnostic.message().to_string(),
            diagnostic.severity(),
            "glue",
        )
    });

    let syntax_is_clean = lexed.diagnostics.is_empty() && parsed.diagnostics.is_empty();
    let elaborated = syntax_is_clean
        .then(|| ir::lower(&parsed.tree, text).diagnostics)
        .unwrap_or_default();
    let semantic = elaborated.iter().map(|diagnostic| {
        (
            diagnostic.span,
            diagnostic.message(),
            diagnostic.severity(),
            "glue (type)",
        )
    });

    lexical
        .chain(grammatical)
        .chain(semantic)
        .map(|(span, message, severity, source)| Diagnostic {
            range: range_of(span, &index),
            severity: Some(match severity {
                tokenizer::Severity::Error => DiagnosticSeverity::ERROR,
                tokenizer::Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            source: Some(source.to_string()),
            message,
            ..Diagnostic::default()
        })
        .collect()
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
            data: semantic::tokens(&parsed.tree, &text, &index),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn of(source: &str) -> Vec<(String, String)> {
        diagnostics_for(source)
            .into_iter()
            .map(|diagnostic| (diagnostic.source.unwrap_or_default(), diagnostic.message))
            .collect()
    }

    #[test]
    fn a_clean_file_says_nothing() {
        assert!(of("let x = 2u64; x * 21u64").is_empty());
    }

    /// The point of the exercise: a type error is an editor squiggle, not a
    /// surprise at run time.
    #[test]
    fn a_type_error_is_reported() {
        let reported = of("let x: Str = 42u64;");
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].0, "glue (type)");
        assert!(reported[0].1.contains("expected `Str`, found `u64`"));
    }

    #[test]
    fn every_elaboration_error_is_reported_not_just_the_first() {
        let reported = of("let a: Str = 1u64; let b: u64 = \"two\";");
        assert_eq!(reported.len(), 2, "{reported:#?}");
        assert!(reported.iter().all(|(source, _)| source == "glue (type)"));
    }

    /// Elaboration waits for a parse. A half-typed line leaves names unbound,
    /// and "no binding named `x`" on top of "expected an expression" explains
    /// nothing that the syntax error did not already say.
    #[test]
    fn type_errors_wait_until_the_file_parses() {
        let reported = of("let x: Str = 42u64; let y = ;");
        assert!(
            reported.iter().all(|(source, _)| source != "glue (type)"),
            "{reported:#?}"
        );
        assert!(reported.iter().any(|(source, _)| source == "glue"));
    }

    /// Lexical and grammatical problems still come out together.
    #[test]
    fn a_lexical_error_does_not_hide_a_grammatical_one() {
        let reported = of("let x = \"unterminated");
        assert!(reported.iter().any(|(source, _)| source == "glue (lex)"));
        assert!(reported.iter().any(|(source, _)| source == "glue"));
    }

    /// A construct the parser accepts and elaboration cannot run reaches the
    /// editor as an ordinary diagnostic rather than silence.
    #[test]
    fn an_unsupported_construct_is_reported() {
        let reported = of("let s = \"hi\"; s[0]");
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].0, "glue (type)");
        assert!(reported[0].1.contains("indexing is not supported yet"));
    }

    /// Today's rule, and the one most likely to be typed by accident.
    #[test]
    fn reading_a_global_too_early_is_reported() {
        let reported = of("let x = foo(); fn foo() -> u64 { y } let y = 1u64; x");
        assert_eq!(reported.len(), 1, "{reported:#?}");
        assert!(reported[0].1.contains("not initialized until later"));
    }
}
