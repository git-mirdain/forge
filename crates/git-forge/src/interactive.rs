//! Interactive prompts for forge commands.

use inquire::{Editor, Select, Text};

use crate::Result;
use crate::error::Error;
use crate::issue::{Issue, IssueState};
use crate::review::{Review, ReviewState};

/// Collected input from the interactive `issue new` prompts.
pub struct NewIssueInput {
    /// Issue title.
    pub title: String,
    /// Issue body in Markdown.
    pub body: String,
    /// Labels to attach.
    pub labels: Vec<String>,
    /// Contributor IDs to assign.
    pub assignees: Vec<String>,
}

/// Collected input from the interactive `issue edit` prompts.
pub struct EditIssueInput {
    /// New title, if changed.
    pub title: Option<String>,
    /// New body, if changed.
    pub body: Option<String>,
    /// New state, if changed.
    pub state: Option<IssueState>,
}

/// Prompt for all fields needed to create a new issue.
///
/// `title_hint` pre-fills the title field (e.g. from a CLI positional).
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels any prompt.
pub fn prompt_new_issue(title_hint: Option<&str>) -> Result<NewIssueInput> {
    let title = Text::new("Title")
        .with_initial_value(title_hint.unwrap_or(""))
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let body = Editor::new("Body")
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let labels_raw = Text::new("Labels (comma-separated)")
        .with_default("")
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let labels = parse_csv(&labels_raw);

    let assignees_raw = Text::new("Assignees (comma-separated)")
        .with_default("")
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let assignees = parse_csv(&assignees_raw);

    Ok(NewIssueInput {
        title,
        body,
        labels,
        assignees,
    })
}

/// Prompt for fields to update on an existing issue, pre-filled with current values.
///
/// Only fields that differ from `current` are returned as `Some`.
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels any prompt.
pub fn prompt_edit_issue(current: &Issue) -> Result<EditIssueInput> {
    let title = Text::new("Title")
        .with_initial_value(&current.title)
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let title = (title != current.title).then_some(title);

    let body = Editor::new("Body")
        .with_predefined_text(&current.body)
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let body = (body != current.body).then_some(body);

    let options = vec!["open", "closed"];
    let default_idx = usize::from(current.state == IssueState::Closed);
    let state_str = Select::new("State", options)
        .with_starting_cursor(default_idx)
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let new_state: IssueState = state_str.parse()?;
    let state = (new_state != current.state).then_some(new_state);

    Ok(EditIssueInput { title, body, state })
}

/// Collected input from the interactive `review new` prompts.
pub struct NewReviewInput {
    /// Review title.
    pub title: String,
    /// Review body in Markdown.
    pub body: String,
}

/// Collected input from the interactive `review edit` prompts.
pub struct EditReviewInput {
    /// New title, if changed.
    pub title: Option<String>,
    /// New body, if changed.
    pub body: Option<String>,
    /// New state, if changed.
    pub state: Option<ReviewState>,
}

/// Prompt for title and description when creating a new review.
///
/// `title_hint` pre-fills the title field.
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels any prompt.
pub fn prompt_new_review(title_hint: Option<&str>) -> Result<NewReviewInput> {
    let title = Text::new("Title")
        .with_initial_value(title_hint.unwrap_or(""))
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let body = Editor::new("Description")
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    Ok(NewReviewInput { title, body })
}

/// Prompt for fields to update on an existing review, pre-filled with current values.
///
/// Only fields that differ from `current` are returned as `Some`.
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels any prompt.
pub fn prompt_edit_review(current: &Review) -> Result<EditReviewInput> {
    let title = Text::new("Title")
        .with_initial_value(&current.title)
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let title = (title != current.title).then_some(title);

    let body = Editor::new("Description")
        .with_predefined_text(&current.body)
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let body = (body != current.body).then_some(body);

    let options = vec!["open", "draft", "closed", "merged"];
    let default_idx = match current.state {
        ReviewState::Open => 0,
        ReviewState::Draft => 1,
        ReviewState::Closed => 2,
        ReviewState::Merged => 3,
    };
    let state_str = Select::new("State", options)
        .with_starting_cursor(default_idx)
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let new_state: ReviewState = state_str.parse()?;
    let state = (new_state != current.state).then_some(new_state);

    Ok(EditReviewInput { title, body, state })
}

/// Prompt for a comment body using an editor.
///
/// `hint` pre-fills the editor (e.g. for editing an existing message).
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels the prompt.
pub fn prompt_body(hint: Option<&str>) -> Result<String> {
    let mut editor = Editor::new("Body");
    if let Some(text) = hint {
        editor = editor.with_predefined_text(text);
    }
    editor.prompt().map_err(|_| Error::Interrupted)
}

fn parse_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}
