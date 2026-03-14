//! Execution logic for `git forge review`.

use crate::cli::ReviewCommand;

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
