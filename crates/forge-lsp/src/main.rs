//! Forge LSP server — surfaces inline comments as inlay hints.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::SystemTime;

use git2::Repository;
use serde_json::Value;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request::ShowDocument;
use tower_lsp::lsp_types::{
    CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability, CodeActionResponse,
    Command, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, ExecuteCommandOptions, ExecuteCommandParams, InitializeParams,
    InitializeResult, InitializedParams, InlayHint, InlayHintLabel, InlayHintParams, MessageType,
    OneOf, Position, SaveOptions, ServerCapabilities, ShowDocumentParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url, WorkDoneProgressOptions,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use git_forge::comment::{
    Anchor, Comment, create_thread, find_thread_by_comment, find_threads_by_object,
    list_thread_comments, reply_to_thread,
};

enum PendingComment {
    New { blob_oid: String, line: u32 },
    Reply { comment_oid: String },
}

struct ForgeLanguageServer {
    client: Client,
    repo_path: RwLock<Option<std::path::PathBuf>>,
    file_contents: RwLock<HashMap<Url, String>>,
    pending: Mutex<HashMap<std::path::PathBuf, PendingComment>>,
}

impl ForgeLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            repo_path: RwLock::new(None),
            file_contents: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
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

    async fn handle_forge_save(&self, path: &std::path::Path, text: Option<String>) {
        let pending = {
            let Ok(mut map) = self.pending.lock() else {
                return;
            };
            map.remove(path)
        };
        let Some(pending) = pending else { return };

        let raw = text.unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default());
        let body: String = raw
            .lines()
            .skip_while(|l| l.starts_with("<!--"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();

        let _ = std::fs::remove_file(path);

        if body.is_empty() {
            return;
        }

        let Some(repo) = self.open_repo() else { return };

        match pending {
            PendingComment::New { blob_oid, line } => {
                let anchor = Anchor {
                    oid: blob_oid,
                    start_line: Some(line + 1),
                    end_line: Some(line + 1),
                };
                let _ = create_thread(&repo, &body, Some(&anchor), None);
            }
            PendingComment::Reply { comment_oid } => {
                if let Ok(Some(thread_id)) = find_thread_by_comment(&repo, &comment_oid) {
                    let _ = reply_to_thread(&repo, &thread_id, &body, &comment_oid, None, None);
                }
            }
        }

        self.refresh().await;
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
        position: Position {
            line,
            character: u32::MAX,
        },
        label: InlayHintLabel::String(label),
        kind: None,
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: Some(Value::String(comment.oid.clone())),
    }
}

fn forge_tmp_path() -> std::path::PathBuf {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("forge-comment-{ts}.md"))
}

fn is_forge_tmp(path: &std::path::Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n.starts_with("forge-comment-")
            && std::path::Path::new(n)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    })
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
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "forge.comment.new".to_string(),
                        "forge.comment.reply".to_string(),
                    ],
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
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
        if let Some(path) = params
            .text_document
            .uri
            .to_file_path()
            .ok()
            .filter(|p| is_forge_tmp(p))
        {
            self.handle_forge_save(&path, params.text).await;
            return;
        }
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

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let line = params.range.start.line;
        let uri = &params.text_document.uri;

        let Some(repo) = self.open_repo() else {
            return Ok(None);
        };
        let Some(content) = self.get_content(uri) else {
            return Ok(None);
        };
        let Some(blob_oid) = Self::hash_content(&repo, content.as_bytes()) else {
            return Ok(None);
        };

        let mut actions: CodeActionResponse = Vec::new();

        actions.push(CodeActionOrCommand::Command(Command {
            title: "New forge comment".to_string(),
            command: "forge.comment.new".to_string(),
            arguments: Some(vec![
                Value::String(uri.to_string()),
                Value::from(line),
                Value::String(blob_oid.clone()),
            ]),
        }));

        for hint in Self::hints_for_blob(&repo, &blob_oid) {
            if hint.position.line == line
                && let Some(Value::String(comment_oid)) = hint.data
            {
                actions.push(CodeActionOrCommand::Command(Command {
                    title: "Reply to forge comment".to_string(),
                    command: "forge.comment.reply".to_string(),
                    arguments: Some(vec![Value::String(comment_oid)]),
                }));
            }
        }

        Ok(Some(actions))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        let args = params.arguments;
        match params.command.as_str() {
            "forge.comment.new" => {
                let Some(Value::String(uri_str)) = args.first() else {
                    return Ok(None);
                };
                let Some(line) = args.get(1).and_then(|v: &Value| v.as_u64()) else {
                    return Ok(None);
                };
                let Some(Value::String(blob_oid)) = args.get(2) else {
                    return Ok(None);
                };

                let Ok(uri) = uri_str.parse::<Url>() else {
                    return Ok(None);
                };
                let path_str = uri
                    .to_file_path()
                    .map_or_else(|()| uri_str.clone(), |p| p.display().to_string());

                let tmp = forge_tmp_path();
                let header = format!(
                    "<!-- forge: new comment on {}:{} -->\n\n",
                    path_str,
                    line + 1
                );
                if std::fs::write(&tmp, &header).is_err() {
                    return Ok(None);
                }
                if let Ok(mut map) = self.pending.lock() {
                    map.insert(
                        tmp.clone(),
                        PendingComment::New {
                            blob_oid: blob_oid.clone(),
                            line: u32::try_from(line).unwrap_or(0),
                        },
                    );
                }
                let Ok(tmp_uri) = Url::from_file_path(&tmp) else {
                    return Ok(None);
                };
                let _ = self
                    .client
                    .send_request::<ShowDocument>(ShowDocumentParams {
                        uri: tmp_uri,
                        external: None,
                        take_focus: Some(true),
                        selection: None,
                    })
                    .await;
            }
            "forge.comment.reply" => {
                let Some(Value::String(comment_oid)) = args.first() else {
                    return Ok(None);
                };

                let tmp = forge_tmp_path();
                let header = format!("<!-- forge: reply to {comment_oid} -->\n\n");
                if std::fs::write(&tmp, &header).is_err() {
                    return Ok(None);
                }
                if let Ok(mut map) = self.pending.lock() {
                    map.insert(
                        tmp.clone(),
                        PendingComment::Reply {
                            comment_oid: comment_oid.clone(),
                        },
                    );
                }
                let Ok(tmp_uri) = Url::from_file_path(&tmp) else {
                    return Ok(None);
                };
                let _ = self
                    .client
                    .send_request::<ShowDocument>(ShowDocumentParams {
                        uri: tmp_uri,
                        external: None,
                        take_focus: Some(true),
                        selection: None,
                    })
                    .await;
            }
            _ => {}
        }
        Ok(None)
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(ForgeLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
