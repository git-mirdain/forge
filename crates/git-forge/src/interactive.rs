//! Interactive prompts for forge commands.

use inquire::{Editor, MultiSelect, Select, Text};

use crate::Result;
use crate::contributor::Contributor;
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

/// Collected input from the interactive `contributor init` prompts.
pub struct InitContributorInput {
    /// Chosen handle.
    pub handle: String,
    /// Display names.
    pub names: Vec<String>,
    /// Email addresses.
    pub emails: Vec<String>,
}

/// Prompt for contributor init fields, pre-filled from git identity.
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels any prompt.
pub fn prompt_init_contributor(
    default_handle: &str,
    default_name: &str,
    default_email: &str,
) -> Result<InitContributorInput> {
    let handle = Text::new("Handle")
        .with_initial_value(default_handle)
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let name = Text::new("Display name")
        .with_initial_value(default_name)
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let email = Text::new("Email")
        .with_initial_value(default_email)
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    Ok(InitContributorInput {
        handle,
        names: vec![name],
        emails: vec![email],
    })
}

/// Edits collected from the interactive contributor edit picker.
pub struct EditContributorInput {
    /// Names to add.
    pub add_names: Vec<String>,
    /// Names to remove.
    pub remove_names: Vec<String>,
    /// Emails to add.
    pub add_emails: Vec<String>,
    /// Emails to remove.
    pub remove_emails: Vec<String>,
    /// Roles to add.
    pub add_roles: Vec<String>,
    /// Roles to remove.
    pub remove_roles: Vec<String>,
}

/// Prompt the user to pick which contributor fields to edit, then prompt for
/// values to add/remove per field.
///
/// # Errors
/// Returns [`Error::Interrupted`] if the user cancels any prompt.
pub fn prompt_edit_contributor(current: &Contributor) -> Result<EditContributorInput> {
    let field_options = vec!["names", "emails", "roles"];
    let fields = MultiSelect::new("Fields to edit", field_options)
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let mut input = EditContributorInput {
        add_names: Vec::new(),
        remove_names: Vec::new(),
        add_emails: Vec::new(),
        remove_emails: Vec::new(),
        add_roles: Vec::new(),
        remove_roles: Vec::new(),
    };

    for field in &fields {
        match *field {
            "names" => {
                let (add, remove) = prompt_add_remove_list("name", &current.names)?;
                input.add_names = add;
                input.remove_names = remove;
            }
            "emails" => {
                let (add, remove) = prompt_add_remove_list("email", &current.emails)?;
                input.add_emails = add;
                input.remove_emails = remove;
            }
            "roles" => {
                let (add, remove) = prompt_add_remove_list("role", &current.roles)?;
                input.add_roles = add;
                input.remove_roles = remove;
            }
            _ => {}
        }
    }

    Ok(input)
}

/// For a given field, show current values as a multi-select (checked = keep,
/// unchecked = remove), then prompt for new values to add.
fn prompt_add_remove_list(label: &str, current: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut remove = Vec::new();
    if !current.is_empty() {
        let defaults: Vec<usize> = (0..current.len()).collect();
        let kept = MultiSelect::new(
            &format!("Current {label}s (deselect to remove)"),
            current.to_vec(),
        )
        .with_default(&defaults)
        .prompt()
        .map_err(|_| Error::Interrupted)?;

        for item in current {
            if !kept.contains(item) {
                remove.push(item.clone());
            }
        }
    }

    let add_raw = Text::new(&format!("New {label}s to add (comma-separated)"))
        .with_default("")
        .prompt()
        .map_err(|_| Error::Interrupted)?;
    let add = parse_csv(&add_raw);

    Ok((add, remove))
}

/// Collected input from the interactive `config edit` prompts.
pub struct EditConfigInput {
    /// Sigils to set (entity → prefix).
    pub add_sigils: Vec<(String, String)>,
    /// Sigil entity names to remove.
    pub remove_sigils: Vec<String>,
    /// Sync scopes to enable.
    pub add_sync: Vec<String>,
    /// Sync scopes to disable.
    pub remove_sync: Vec<String>,
}

/// Prompt the user to edit a provider config entry.
///
/// # Errors
/// Returns an error if a prompt is interrupted.
pub fn prompt_edit_config(
    current_sigils: &std::collections::BTreeMap<String, String>,
    current_sync: &std::collections::BTreeMap<String, String>,
) -> Result<EditConfigInput> {
    let field_options = vec!["sigils", "sync"];
    let fields = MultiSelect::new("Fields to edit", field_options)
        .prompt()
        .map_err(|_| Error::Interrupted)?;

    let mut input = EditConfigInput {
        add_sigils: Vec::new(),
        remove_sigils: Vec::new(),
        add_sync: Vec::new(),
        remove_sync: Vec::new(),
    };

    for field in &fields {
        match *field {
            "sigils" => {
                let current_entries: Vec<String> = current_sigils
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                if !current_entries.is_empty() {
                    let defaults: Vec<usize> = (0..current_entries.len()).collect();
                    let kept = MultiSelect::new(
                        "Current sigils (deselect to remove)",
                        current_entries.clone(),
                    )
                    .with_default(&defaults)
                    .prompt()
                    .map_err(|_| Error::Interrupted)?;

                    let keys: Vec<&String> = current_sigils.keys().collect();
                    for (i, entry) in current_entries.iter().enumerate() {
                        if !kept.contains(entry) {
                            input.remove_sigils.push(keys[i].clone());
                        }
                    }
                }
                let add_raw =
                    Text::new("New sigils to add (comma-separated, e.g. issue=GH#,review=PR#)")
                        .with_default("")
                        .prompt()
                        .map_err(|_| Error::Interrupted)?;
                for pair in parse_csv(&add_raw) {
                    if let Some((k, v)) = pair.split_once('=') {
                        input.add_sigils.push((k.to_string(), v.to_string()));
                    }
                }
            }
            "sync" => {
                let all_scopes = vec!["issues", "reviews"];
                let defaults: Vec<usize> = all_scopes
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| current_sync.contains_key(**s))
                    .map(|(i, _)| i)
                    .collect();
                let selected =
                    MultiSelect::new("Sync scopes (select to enable)", all_scopes.clone())
                        .with_default(&defaults)
                        .prompt()
                        .map_err(|_| Error::Interrupted)?;

                for scope in &all_scopes {
                    let was_enabled = current_sync.contains_key(*scope);
                    let is_enabled = selected.contains(scope);
                    if is_enabled && !was_enabled {
                        input.add_sync.push((*scope).to_string());
                    } else if !is_enabled && was_enabled {
                        input.remove_sync.push((*scope).to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(input)
}

fn parse_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}
