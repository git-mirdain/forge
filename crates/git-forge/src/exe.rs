//! Execution logic for `git forge` subcommands.

use crate::cli::issue::IssueCommand;
use crate::cli::review::ReviewCommand;

pub mod issue {
    //! Execution logic for `git forge issue`.

    use super::IssueCommand;

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
}

pub mod review {
    //! Execution logic for `git forge review`.

    use super::ReviewCommand;

    /// Execute a `review` subcommand.
    pub fn run(command: ReviewCommand) {
        match command {
            ReviewCommand::New => todo!(),
            ReviewCommand::Edit => todo!(),
            ReviewCommand::List => todo!(),
            ReviewCommand::Status => todo!(),
            ReviewCommand::Show => todo!(),
        }
    }
}
