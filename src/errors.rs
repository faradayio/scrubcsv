//! Error-handling support implemented using the `[error-chain][]` crate.
//!
//! [error-chain]: https://docs.rs/error-chain

use csv;
use std::io;

// Declare nicer `Error` and `Result` types.  This is a macro that
// generates a lot of boilerplate code for us.
error_chain! {
    // Error types from other libraries that we want to just wrap
    // automatically.
    foreign_links {
        Csv(csv::Error);
        Io(io::Error);
    }

    // Our custom error types.
    errors {
        TooManyBadRows(bad: u64, total: u64) {
            description("a large portion of your rows were bad")
            display("a large portion of your rows ({} of {}) were bad", bad, total)
        }
    }
}

// Add custom methods to our `Error` type.
impl Error {
    /// Should we show a backtrace for this particular error?
    pub fn should_show_backtrace(&self) -> bool {
        match *self.kind() {
            ErrorKind::TooManyBadRows(_, _) => false,
            _ => true,
        }
    }

    /// What exit code should we return when the process exits?
    pub fn to_exit_code(&self) -> i32 {
        match *self.kind() {
            // This is only arguably an error, so return a special code for
            // people who want to try to ignore it.
            ErrorKind::TooManyBadRows(_, _) => 2,
            _ => 1,
        }
    }
}
