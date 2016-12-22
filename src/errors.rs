//! Error-handling support implemented using the `[error-chain][]` crate.
//!
//! [error-chain]: https://docs.rs/error-chain

use csv;
use std::io;

// Declare nicer `Error` and `Result` types.
error_chain! {
    // Error types from other libraries that we want to just wrap
    // automatically.
    foreign_links {
        Csv(csv::Error);
        Io(io::Error);
    }
}
