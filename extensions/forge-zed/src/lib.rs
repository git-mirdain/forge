use zed_extension_api::{self as zed, Command, ContextServerId, LanguageServerId, Project, Result};

struct ForgeExtension;

impl zed::Extension for ForgeExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Command> {
        const BINARY_NAME: &str = "forge-lsp";

        let candidate = format!("{}/target/debug/{BINARY_NAME}", worktree.root_path());
        if std::fs::metadata(&candidate).is_ok() {
            return Ok(Command {
                command: candidate,
                args: vec![],
                env: vec![],
            });
        }

        if let Some(path) = worktree.which(BINARY_NAME) {
            return Ok(Command {
                command: path,
                args: vec![],
                env: vec![],
            });
        }

        Ok(Command {
            command: BINARY_NAME.to_string(),
            args: vec![],
            env: vec![],
        })
    }

    fn context_server_command(
        &mut self,
        _context_server_id: &ContextServerId,
        project: &Project,
    ) -> Result<Command> {
        let binary_name = "forge-mcp";

        // Look for the binary in each worktree: first check target/debug, then $PATH.
        for worktree_id in project.worktree_ids() {
            let worktree = unsafe { zed::Worktree::from_handle(worktree_id as u32) };
            let candidate = format!("{}/target/debug/{binary_name}", worktree.root_path());
            if std::fs::metadata(&candidate).is_ok() {
                return Ok(Command {
                    command: candidate,
                    args: vec![],
                    env: vec![],
                });
            }
            if let Some(path) = worktree.which(binary_name) {
                return Ok(Command {
                    command: path,
                    args: vec![],
                    env: vec![],
                });
            }
        }

        // Last resort: bare name and hope the runtime resolves it.
        Ok(Command {
            command: binary_name.to_string(),
            args: vec![],
            env: vec![],
        })
    }
}

zed::register_extension!(ForgeExtension);
