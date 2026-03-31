//! Forge LSP server — surfaces inline comments as diagnostics.

use std::sync::RwLock;

use git2::Repository;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams, InitializeResult,
    InitializedParams, MessageType, Position, Range, SaveOptions, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use git_forge::comment::{Comment, find_threads_by_object, list_thread_comments};

struct ForgeLanguageServer {
    client: Client,
    repo_path: RwLock<Option<std::path::PathBuf>>,
}

impl ForgeLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            repo_path: RwLock::new(None),
        }
    }

    fn open_repo(&self) -> Option<Repository> {
        let path = self.repo_path.read().ok()?;
        let path = path.as_ref()?;
        Repository::discover(path).ok()
    }

    fn hash_content(repo: &Repository, content: &[u8]) -> Option<String> {
        repo.blob(content).ok().map(|oid| oid.to_string())
    }

    fn diagnostics_for_blob(repo: &Repository, blob_oid: &str) -> Vec<Diagnostic> {
        let Ok(thread_ids) = find_threads_by_object(repo, blob_oid) else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        for thread_id in &thread_ids {
            let Ok(comments) = list_thread_comments(repo, thread_id) else {
                continue;
            };
            for c in comments
                .iter()
                .filter(|c| !c.resolved && c.replaces.is_none())
            {
                let range = comment_range(c);
                let message = format!("{}: {}", c.author_name, c.body);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::HINT),
                    source: Some("forge".into()),
                    message,
                    ..Default::default()
                });
            }
        }
        diagnostics
    }

    async fn publish_diagnostics(&self, uri: &Url, content: &str) {
        let diagnostics = self
            .open_repo()
            .and_then(|repo| {
                let blob_oid = Self::hash_content(&repo, content.as_bytes())?;
                Some(Self::diagnostics_for_blob(&repo, &blob_oid))
            })
            .unwrap_or_default();

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

fn comment_range(comment: &Comment) -> Range {
    if let Some(anchor) = &comment.anchor
        && let Some(start) = anchor.start_line
    {
        let start_line = start.saturating_sub(1);
        let end_line = anchor.end_line.unwrap_or(start).saturating_sub(1);
        return Range {
            start: Position {
                line: start_line,
                character: 0,
            },
            end: Position {
                line: end_line,
                character: u32::MAX,
            },
        };
    }
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: u32::MAX,
        },
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for ForgeLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(root) = params.root_uri.as_ref().and_then(|u| u.to_file_path().ok())
            && let Ok(mut path) = self.repo_path.write()
        {
            *path = Some(root);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "forge-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.publish_diagnostics(&params.text_document.uri, &params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.last() {
            self.publish_diagnostics(&params.text_document.uri, &change.text)
                .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = &params.text {
            self.publish_diagnostics(&params.text_document.uri, text)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(ForgeLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
