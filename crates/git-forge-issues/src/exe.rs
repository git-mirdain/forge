//! Execution logic for `git forge issue`.

use crate::cli::IssueCommand;

/// Execute an `issue` subcommand.
pub fn run(command: IssueCommand) {
    match command {
        IssueCommand::New => todo!(),
        IssueCommand::Edit => todo!(),
        IssueCommand::List => todo!(),
        IssueCommand::Status => todo!(),
        IssueCommand::Show => todo!(),
    }
}
