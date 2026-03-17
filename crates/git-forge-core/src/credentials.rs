//! Credential-aware fetch and push options for git2 remotes.

use git2_credentials::CredentialHandler;

/// Build [`git2::FetchOptions`] wired to the user's credential helpers.
pub fn fetch_options() -> Result<git2::FetchOptions<'static>, Box<dyn std::error::Error>> {
    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(credential_callbacks()?);
    Ok(opts)
}

/// Build [`git2::PushOptions`] wired to the user's credential helpers.
pub fn push_options() -> Result<git2::PushOptions<'static>, Box<dyn std::error::Error>> {
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(credential_callbacks()?);
    Ok(opts)
}

// TODO audit: open_default may not respect the repo-local config; consider
// passing the repo's config or snapshot so per-repo credential helpers apply.
// TODO audit: CredentialHandler::new enables dialoguer interactive prompts as
// a last resort — verify this is acceptable for non-interactive / scripted use.
fn credential_callbacks() -> Result<git2::RemoteCallbacks<'static>, Box<dyn std::error::Error>> {
    let git_config = git2::Config::open_default()?;
    let mut ch = CredentialHandler::new(git_config);
    let mut cb = git2::RemoteCallbacks::new();
    cb.credentials(move |url, username, allowed| ch.try_next_credential(url, username, allowed));
    Ok(cb)
}
