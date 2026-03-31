//! Forge LSP server — surfaces inline comments as inlay hints.

use std::collections::HashMap;
use std::sync::RwLock;

use git2::Repository;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializeParams, InitializeResult, InitializedParams, InlayHint,
    InlayHintLabel, InlayHintParams, MessageType, OneOf, Position, SaveOptions, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use git_forge::comment::{Comment, find_threads_by_object, list_thread_comments};

struct ForgeLanguageServer {
    client: Client,
    repo_path: RwLock<Option<std::path::PathBuf>>,
    file_contents: RwLock<HashMap<Url, String>>,
}

impl ForgeLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            repo_path: RwLock::new(None),
            file_contents: RwLock::new(HashMap::new()),
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

    fn hints_for_blob(repo: &Repository, blob_oid: &str) -> Vec<InlayHint> {
        let Ok(thread_ids) = find_threads_by_object(repo, blob_oid) else {
            return Vec::new();
        };

        let mut hints = Vec::new();
        for thread_id in &thread_ids {
            let Ok(comments) = list_thread_comments(repo, thread_id) else {
                continue;
            };
            for c in comments
                .iter()
                .filter(|c| !c.resolved && c.replaces.is_none())
            {
                hints.push(hint_for_comment(c));
            }
        }
        hints
    }

    fn store_content(&self, uri: Url, content: String) {
        if let Ok(mut map) = self.file_contents.write() {
            map.insert(uri, content);
        }
    }

    fn remove_content(&self, uri: &Url) {
        if let Ok(mut map) = self.file_contents.write() {
            map.remove(uri);
        }
    }

    fn get_content(&self, uri: &Url) -> Option<String> {
        self.file_contents.read().ok()?.get(uri).cloned()
    }

    async fn refresh(&self) {
        let _ = self.client.inlay_hint_refresh().await;
    }
}

fn hint_for_comment(comment: &Comment) -> InlayHint {
    let line = comment
        .anchor
        .as_ref()
        .and_then(|a| a.start_line)
        .map_or(0, |l| l.saturating_sub(1));
    let first_line = comment.body.lines().next().unwrap_or(&comment.body);
    let label = format!("▸ {}: {}", comment.author_name, first_line);
    InlayHint {
        position: Position { line, character: 0 },
        label: InlayHintLabel::String(label),
        kind: None,
        text_edits: None,
        tooltip: None,
        padding_left: None,
        padding_right: Some(true),
        data: None,
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
                inlay_hint_provider: Some(OneOf::Left(true)),
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
        self.store_content(params.text_document.uri, params.text_document.text);
        self.refresh().await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.last() {
            self.store_content(params.text_document.uri, change.text.clone());
            self.refresh().await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = params.text {
            self.store_content(params.text_document.uri, text);
            self.refresh().await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.remove_content(&params.text_document.uri);
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let hints = self
            .open_repo()
            .and_then(|repo| {
                let content = self.get_content(&params.text_document.uri)?;
                let blob_oid = Self::hash_content(&repo, content.as_bytes())?;
                Some(Self::hints_for_blob(&repo, &blob_oid))
            })
            .unwrap_or_default();

        Ok(Some(hints))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(ForgeLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
