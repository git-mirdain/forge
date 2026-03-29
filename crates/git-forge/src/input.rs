//! Resolve body/content from CLI flags, file, or stdin pipe.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

/// Resolve body content with precedence: explicit value > file > stdin pipe.
///
/// Returns `None` when no source provided and stdin is a terminal (letting
/// callers fall through to interactive prompts).
///
/// # Errors
/// Returns an error if the file cannot be read or stdin cannot be consumed.
pub fn resolve_body(
    body: Option<String>,
    file: Option<PathBuf>,
) -> std::io::Result<Option<String>> {
    if body.is_some() {
        return Ok(body);
    }
    if let Some(path) = file {
        return Ok(Some(std::fs::read_to_string(path)?));
    }
    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return Ok(Some(buf));
    }
    Ok(None)
}
