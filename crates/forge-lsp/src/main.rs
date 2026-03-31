//! Forge LSP server — surfaces inline comments as inlay hints.

use std::collections::HashMap;
use std::sync::RwLock;

use git2::Repository;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializeParams, InitializeResult, InitializedParams, InlayHint,
    InlayHintLabel, InlayHintOptions, InlayHintParams, InlayHintServerCapabilities, MessageType,
    OneOf, Position, SaveOptions, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, Url,
    WorkDoneProgressOptions,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use git_forge::comment::{self, Anchor, Comment, object_comment_ref};

struct ForgeLanguageServer {
    client: Client,
    repo_path: RwLock<Option<std::path::PathBuf>>,
    content_cache: RwLock<HashMap<Url, String>>,
}

impl ForgeLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            repo_path: RwLock::new(None),
            content_cache: RwLock::new(HashMap::new()),
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
        let ref_name = object_comment_ref(blob_oid);
        let Ok(comments) = comment::list_comments(repo, &ref_name) else {
            return Vec::new();
        };

        comments
            .iter()
            .filter(|c| !c.resolved && c.replaces.is_none())
            .map(|c| {
                let position = comment_position(c);
                let label = format!("▸ {}: {}", c.author_name, first_line(&c.body));
                InlayHint {
                    position,
                    label: InlayHintLabel::String(label),
                    kind: None,
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                }
            })
            .collect()
    }

    async fn store_and_refresh(&self, uri: &Url, content: &str) {
        if let Ok(mut cache) = self.content_cache.write() {
            cache.insert(uri.clone(), content.to_string());
        }
        self.client.inlay_hint_refresh().await.ok();
    }
}

#[allow(clippy::cast_possible_truncation)]
fn comment_position(comment: &Comment) -> Position {
    if let Some(Anchor::Object {
        range: Some(range), ..
    }) = &comment.anchor
        && let Some((start, _)) = parse_line_range(range)
    {
        return Position {
            line: start.saturating_sub(1) as u32,
            character: u32::MAX,
        };
    }
    Position {
        line: 0,
        character: u32::MAX,
    }
}

fn parse_line_range(range: &str) -> Option<(usize, usize)> {
    let (a, b) = range.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
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
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        resolve_provider: Some(false),
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    },
                ))),
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
        self.store_and_refresh(&params.text_document.uri, &params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.last() {
            self.store_and_refresh(&params.text_document.uri, &change.text)
                .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = &params.text {
            self.store_and_refresh(&params.text_document.uri, text)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if let Ok(mut cache) = self.content_cache.write() {
            cache.remove(&params.text_document.uri);
        }
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let Some(repo) = self.open_repo() else {
            return Ok(None);
        };

        let uri = &params.text_document.uri;
        let content = {
            let cache = self.content_cache.read().ok();
            cache.and_then(|c| c.get(uri).cloned())
        };
        let Some(content) = content else {
            return Ok(None);
        };

        let Some(blob_oid) = Self::hash_content(&repo, content.as_bytes()) else {
            return Ok(None);
        };

        Ok(Some(Self::hints_for_blob(&repo, &blob_oid)))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(ForgeLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
