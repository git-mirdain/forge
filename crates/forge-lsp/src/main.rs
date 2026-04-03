//! Forge LSP server — surfaces inline comments as diagnostics, inlay hints,
//! and code actions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use git2::Repository;
use serde_json::Value;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability,
    CodeActionResponse, Command, CreateFile, CreateFileOptions, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, DocumentChangeOperation, DocumentChanges, ExecuteCommandOptions,
    ExecuteCommandParams, InitializeParams, InitializeResult, InitializedParams, InlayHint,
    InlayHintLabel, InlayHintParams, MessageType, NumberOrString, OneOf,
    OptionalVersionedTextDocumentIdentifier, Position, Range, ResourceOp, SaveOptions,
    ServerCapabilities, ShowDocumentParams, TextDocumentEdit, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Url,
    WorkDoneProgressOptions, WorkspaceEdit,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use git_forge::comment::{
    Anchor, Comment, create_thread, find_thread_by_comment, find_threads_by_object,
    list_thread_comments, reply_to_thread, resolve_thread,
};
use git_forge::refs::walk_tree;

enum PendingComment {
    New { blob_oid: String, line: u32 },
    Reply { comment_oid: String },
}

const DRAFTS_DIR: &str = ".git/forge/comments/drafts";
const DIAGNOSTIC_SOURCE: &str = "forge";

struct ForgeLanguageServer {
    client: Client,
    repo_path: RwLock<Option<PathBuf>>,
    file_contents: RwLock<HashMap<Url, String>>,
    pending: Mutex<HashMap<PathBuf, PendingComment>>,
    published_uris: Mutex<HashSet<Url>>,
    draft_counter: AtomicU64,
}

// ── comment collection ───────────────────────────────────────────────────────

/// A comment with its owning thread ID, for diagnostics and code actions.
struct ThreadComment {
    thread_id: String,
    comment: Comment,
}

/// Walk the HEAD tree and collect all `(relative_path, blob_oid)` pairs.
fn head_blob_map(repo: &Repository) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(head) = repo.head() else {
        return map;
    };
    let Ok(commit) = head.peel_to_commit() else {
        return map;
    };
    let Ok(tree) = commit.tree() else {
        return map;
    };
    let mut files = Vec::new();
    walk_tree(repo, &tree, "", &mut files);
    for (path, oid) in files {
        map.entry(oid).or_default().push(path);
    }
    map
}

/// Collect all unresolved thread comments anchored to a given blob OID.
fn comments_for_blob(repo: &Repository, blob_oid: &str) -> Vec<ThreadComment> {
    let Ok(thread_ids) = find_threads_by_object(repo, blob_oid) else {
        return Vec::new();
    };
    let mut all = Vec::new();
    for tid in &thread_ids {
        let Ok(comments) = list_thread_comments(repo, tid) else {
            continue;
        };
        for c in comments {
            all.push(ThreadComment {
                thread_id: tid.clone(),
                comment: c,
            });
        }
    }
    let replaced: HashSet<String> = all
        .iter()
        .filter_map(|tc| tc.comment.replaces.clone())
        .collect();
    all.into_iter()
        .filter(|tc| !replaced.contains(&tc.comment.oid))
        .collect()
}

// ── diagnostics ──────────────────────────────────────────────────────────────

fn diagnostic_for_comment(tc: &ThreadComment) -> Diagnostic {
    let c = &tc.comment;
    let line = c
        .anchor
        .as_ref()
        .and_then(|a| a.start_line)
        .map_or(0, |l| l.saturating_sub(1));
    let end_line = c
        .anchor
        .as_ref()
        .and_then(|a| a.end_line)
        .map_or(line, |l| l.saturating_sub(1));

    let first_line = c.body.lines().next().unwrap_or(&c.body);
    let message = format!("{}: {first_line}", c.author_name);

    let severity = if c.resolved {
        DiagnosticSeverity::HINT
    } else {
        DiagnosticSeverity::INFORMATION
    };

    Diagnostic {
        range: Range {
            start: Position { line, character: 0 },
            end: Position {
                line: end_line,
                character: u32::MAX,
            },
        },
        severity: Some(severity),
        source: Some(DIAGNOSTIC_SOURCE.to_string()),
        message,
        code: Some(NumberOrString::String(tc.thread_id.clone())),
        data: Some(Value::String(c.oid.clone())),
        ..Default::default()
    }
}

// ── server impl ──────────────────────────────────────────────────────────────

impl ForgeLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            repo_path: RwLock::new(None),
            file_contents: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            published_uris: Mutex::new(HashSet::new()),
            draft_counter: AtomicU64::new(0),
        }
    }

    // NB: calls `Repository::discover()` on every invocation because
    // `git2::Repository` is `!Sync` and cannot be cached on the server struct.
    fn open_repo(&self) -> Option<Repository> {
        let path = self.repo_path.read().ok()?;
        let path = path.as_ref()?;
        Repository::discover(path).ok()
    }

    fn repo_root(&self) -> Option<PathBuf> {
        let repo = self.open_repo()?;
        Some(
            repo.workdir()
                .map_or_else(|| repo.path().into(), Into::into),
        )
    }

    fn workdir(&self) -> Option<PathBuf> {
        self.open_repo()?.workdir().map(Into::into)
    }

    fn hash_content(_repo: &Repository, content: &[u8]) -> Option<String> {
        git2::Oid::hash_object(git2::ObjectType::Blob, content)
            .ok()
            .map(|oid| oid.to_string())
    }

    fn hints_for_blob(repo: &Repository, blob_oid: &str) -> Vec<InlayHint> {
        let tcs = comments_for_blob(repo, blob_oid);
        tcs.iter()
            .filter(|tc| !tc.comment.resolved)
            .map(|tc| hint_for_comment(&tc.comment))
            .collect()
    }

    fn store_content(&self, uri: Url, content: String) {
        self.file_contents
            .write()
            .expect("lock poisoned")
            .insert(uri, content);
    }

    fn remove_content(&self, uri: &Url) {
        self.file_contents
            .write()
            .expect("lock poisoned")
            .remove(uri);
    }

    fn get_content(&self, uri: &Url) -> Option<String> {
        self.file_contents
            .read()
            .expect("lock poisoned")
            .get(uri)
            .cloned()
    }

    async fn refresh(&self) {
        let _ = self.client.inlay_hint_refresh().await;
    }

    fn draft_path(&self) -> Option<PathBuf> {
        let root = self.repo_root()?;
        let dir = root.join(DRAFTS_DIR);
        let _ = std::fs::create_dir_all(&dir);
        let pid = std::process::id();
        let seq = self.draft_counter.fetch_add(1, Ordering::Relaxed);
        Some(dir.join(format!("forge-comment-{pid}-{seq}.md")))
    }

    fn is_draft(path: &std::path::Path) -> bool {
        let s = path.to_string_lossy();
        s.contains(".git/forge/comments/drafts/")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("forge-comment-"))
    }

    /// Publish diagnostics for all files in the HEAD tree that have comments.
    async fn publish_all_diagnostics(&self) {
        let Some(repo) = self.open_repo() else {
            return;
        };
        let Some(workdir) = self.workdir() else {
            return;
        };

        let blob_map = head_blob_map(&repo);

        // Collect diagnostics grouped by relative path.
        let mut diags_by_path: HashMap<String, Vec<Diagnostic>> = HashMap::new();
        for (blob_oid, paths) in &blob_map {
            let tcs = comments_for_blob(&repo, blob_oid);
            if tcs.is_empty() {
                continue;
            }
            let diagnostics: Vec<Diagnostic> = tcs.iter().map(diagnostic_for_comment).collect();
            for path in paths {
                diags_by_path
                    .entry(path.clone())
                    .or_default()
                    .extend(diagnostics.clone());
            }
        }

        // Publish and track which URIs we published to.
        let mut current_uris = HashSet::new();
        for (path, diagnostics) in &diags_by_path {
            let abs = workdir.join(path);
            if let Ok(uri) = Url::from_file_path(&abs) {
                self.client
                    .publish_diagnostics(uri.clone(), diagnostics.clone(), None)
                    .await;
                current_uris.insert(uri);
            }
        }

        // Clear diagnostics for URIs that were previously published but no
        // longer have any comments.
        let previous = {
            let mut published = self.published_uris.lock().expect("lock poisoned");
            std::mem::replace(&mut *published, current_uris.clone())
        };
        for stale in previous.difference(&current_uris) {
            self.client
                .publish_diagnostics(stale.clone(), Vec::new(), None)
                .await;
        }
    }

    /// Publish diagnostics for a single file using its live (possibly dirty)
    /// blob OID.
    async fn publish_file_diagnostics(&self, uri: &Url) {
        let Some(repo) = self.open_repo() else {
            return;
        };
        let Some(content) = self.get_content(uri) else {
            return;
        };
        let Some(blob_oid) = Self::hash_content(&repo, content.as_bytes()) else {
            return;
        };
        let tcs = comments_for_blob(&repo, &blob_oid);
        let diagnostics: Vec<Diagnostic> = tcs.iter().map(diagnostic_for_comment).collect();
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
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
        let mut lines = raw.lines();
        // Skip the forge header line if present.
        if let Some(first) = lines.clone().next()
            && first.starts_with("<!-- forge:")
        {
            lines.next();
        }
        let body: String = lines.collect::<Vec<_>>().join("\n").trim().to_string();

        if body.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }

        let Some(repo) = self.open_repo() else { return };

        let result: std::result::Result<(), String> = match pending {
            PendingComment::New { blob_oid, line } => {
                let anchor = Anchor {
                    oid: blob_oid,
                    start_line: Some(line + 1),
                    end_line: Some(line + 1),
                };
                create_thread(&repo, &body, Some(&anchor), None)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            PendingComment::Reply { comment_oid } => {
                match find_thread_by_comment(&repo, &comment_oid) {
                    Ok(Some(thread_id)) => {
                        reply_to_thread(&repo, &thread_id, &body, &comment_oid, None, None)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                    Ok(None) => Err(format!("thread not found for {comment_oid}")),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        match result {
            Ok(()) => {
                let _ = std::fs::remove_file(path);
            }
            Err(msg) => {
                self.client
                    .show_message(MessageType::ERROR, format!("forge: {msg}"))
                    .await;
                return;
            }
        }

        self.refresh().await;
        self.publish_all_diagnostics().await;
    }

    async fn open_unresolved_files(&self) {
        let Some(repo) = self.open_repo() else {
            return;
        };
        let Some(workdir) = self.workdir() else {
            return;
        };

        // Collect into BTreeMap for deterministic file ordering.
        let blob_map: BTreeMap<String, Vec<String>> = head_blob_map(&repo).into_iter().collect();
        let mut opened = 0u32;
        for (blob_oid, paths) in &blob_map {
            let tcs = comments_for_blob(&repo, blob_oid);
            let has_unresolved = tcs.iter().any(|tc| !tc.comment.resolved);
            if !has_unresolved {
                continue;
            }
            for path in paths {
                let abs = workdir.join(path);
                let Ok(uri) = Url::from_file_path(&abs) else {
                    continue;
                };
                match self
                    .client
                    .show_document(ShowDocumentParams {
                        uri,
                        external: None,
                        take_focus: Some(opened == 0),
                        selection: None,
                    })
                    .await
                {
                    Ok(_) => opened += 1,
                    Err(e) => {
                        self.client
                            .log_message(
                                MessageType::WARNING,
                                format!("showDocument failed for {path}: {e}"),
                            )
                            .await;
                    }
                }
            }
        }

        if opened == 0 {
            self.client
                .log_message(MessageType::INFO, "No files with unresolved comments")
                .await;
        }
    }

    async fn open_draft(&self, path: &PathBuf, header: &str, pending: PendingComment) {
        let Ok(uri) = Url::from_file_path(path) else {
            return;
        };

        self.pending
            .lock()
            .expect("lock poisoned")
            .insert(path.clone(), pending);

        let edit = WorkspaceEdit {
            changes: None,
            document_changes: Some(DocumentChanges::Operations(vec![
                DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                    uri: uri.clone(),
                    options: Some(CreateFileOptions {
                        overwrite: Some(true),
                        ignore_if_exists: None,
                    }),
                    annotation_id: None,
                })),
                DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier { uri, version: None },
                    edits: vec![OneOf::Left(TextEdit {
                        range: Range {
                            start: Position {
                                line: 0,
                                character: 0,
                            },
                            end: Position {
                                line: 0,
                                character: 0,
                            },
                        },
                        new_text: header.to_string(),
                    })],
                }),
            ])),
            change_annotations: None,
        };

        let _ = self.client.apply_edit(edit).await;
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
                        "forge.comment.resolve".to_string(),
                        "forge.review.open".to_string(),
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
        self.publish_all_diagnostics().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.store_content(params.text_document.uri, params.text_document.text);
        self.refresh().await;
        self.publish_file_diagnostics(&uri).await;
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
            .filter(|p| Self::is_draft(p))
        {
            self.handle_forge_save(&path, params.text).await;
            return;
        }
        if let Some(text) = params.text {
            let uri = params.text_document.uri.clone();
            self.store_content(params.text_document.uri, text);
            self.refresh().await;
            self.publish_file_diagnostics(&uri).await;
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

        let tcs = comments_for_blob(&repo, &blob_oid);
        let has_unresolved = tcs.iter().any(|tc| !tc.comment.resolved);

        if has_unresolved {
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Open all files with unresolved comments".to_string(),
                command: Some(Command {
                    title: "Open all files with unresolved comments".to_string(),
                    command: "forge.review.open".to_string(),
                    arguments: None,
                }),
                ..Default::default()
            }));
        }

        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "New forge comment".to_string(),
            command: Some(Command {
                title: "New forge comment".to_string(),
                command: "forge.comment.new".to_string(),
                arguments: Some(vec![
                    Value::String(uri.to_string()),
                    Value::from(line),
                    Value::String(blob_oid.clone()),
                ]),
            }),
            ..Default::default()
        }));

        for tc in &tcs {
            let c = &tc.comment;
            if c.resolved {
                continue;
            }
            let comment_line = c
                .anchor
                .as_ref()
                .and_then(|a| a.start_line)
                .map_or(0, |l| l.saturating_sub(1));
            let end_line = c
                .anchor
                .as_ref()
                .and_then(|a| a.end_line)
                .map_or(comment_line, |l| l.saturating_sub(1));
            if line < comment_line || line > end_line {
                continue;
            }

            let first_line = c.body.lines().next().unwrap_or(&c.body);
            let truncated: String = first_line.chars().take(40).collect();

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Reply: {truncated}"),
                command: Some(Command {
                    title: "Reply to forge comment".to_string(),
                    command: "forge.comment.reply".to_string(),
                    arguments: Some(vec![Value::String(c.oid.clone())]),
                }),
                ..Default::default()
            }));

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Resolve: {truncated}"),
                command: Some(Command {
                    title: "Resolve forge comment".to_string(),
                    command: "forge.comment.resolve".to_string(),
                    arguments: Some(vec![
                        Value::String(tc.thread_id.clone()),
                        Value::String(c.oid.clone()),
                    ]),
                }),
                ..Default::default()
            }));
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

                let Some(tmp) = self.draft_path() else {
                    return Ok(None);
                };
                let header = format!(
                    "<!-- forge: new comment on {}:{} -->\n\n",
                    path_str,
                    line + 1
                );

                let line = u32::try_from(line).unwrap_or(0);
                self.open_draft(
                    &tmp,
                    &header,
                    PendingComment::New {
                        blob_oid: blob_oid.clone(),
                        line,
                    },
                )
                .await;
            }
            "forge.comment.reply" => {
                let Some(Value::String(comment_oid)) = args.first() else {
                    return Ok(None);
                };

                let Some(tmp) = self.draft_path() else {
                    return Ok(None);
                };
                let header = format!("<!-- forge: reply to {comment_oid} -->\n\n");

                self.open_draft(
                    &tmp,
                    &header,
                    PendingComment::Reply {
                        comment_oid: comment_oid.clone(),
                    },
                )
                .await;
            }
            "forge.review.open" => {
                self.open_unresolved_files().await;
            }
            "forge.comment.resolve" => {
                let Some(Value::String(thread_id)) = args.first() else {
                    return Ok(None);
                };
                let Some(Value::String(comment_oid)) = args.get(1) else {
                    return Ok(None);
                };
                let Some(repo) = self.open_repo() else {
                    return Ok(None);
                };
                let _ = resolve_thread(&repo, thread_id, comment_oid, None);
                self.refresh().await;
                self.publish_all_diagnostics().await;
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
