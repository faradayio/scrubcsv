//! Error-handling support. This emulates libraries like `error-chain` (very
//! deprecated), `failure` (somewhat deprecated) and `snafu` (currently
//! recommended, but I'm starting to see a pattern here).
//!
//! I just wanted to see how hard it was to roll an nice error API from scratch
//! instead of depending on an unstable third-party library.

use std::{error, fmt, result};

/// Our error type. We used a boxed dynamic error because we don't care much
/// about the details and we're only going to print it for the user anyways.
pub type Error = Box<dyn error::Error + 'static>;

/// Our custom `Result` type. Defaults the `E` parameter to our error type.
pub type Result<T, E = Error> = result::Result<T, E>;

/// Human-readable context for another error.
#[derive(Debug)]
pub struct Context {
    context: String,
    source: Error,
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.context.fmt(f)
    }
}

impl error::Error for Context {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Extend `Result` with methods that add context to errors.
pub trait ResultExt<T, E>: Sized {
    /// If this result is an error, wrap that error with `context`.
    fn context<C>(self, context: C) -> Result<T>
    where
        C: Into<String>,
    {
        self.with_context(|_| context.into())
    }

    /// If this result is an error, call `build_context` and wrap the error in
    /// that context.
    fn with_context<C, F>(self, build_context: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce(&E) -> C;
}

impl<T, E: error::Error + 'static> ResultExt<T, E> for Result<T, E> {
    fn with_context<C, F>(self, build_context: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce(&E) -> C,
    {
        self.map_err(|err| {
            Box::new(Context {
                context: build_context(&err).into(),
                source: Box::new(err),
            }) as Error
        })
    }
}

/// Format a string and return it as an `Error`. We use a macro to do this,
/// because that's the only way to declare `format!`-like syntax in Rust.
#[macro_export]
macro_rules! format_err {
    ($format_str:literal) => ({
        let err: $crate::errors::Error = format!($format_str).into();
        err
    });
    ($format_str:literal, $($arg:expr),*) => ({
        let err: $crate::errors::Error = format!($format_str, $($arg),*).into();
        err
    });
}
