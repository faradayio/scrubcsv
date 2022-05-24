//! Miscellaneous utilities.

use std::str::FromStr;
use time::{Duration, OffsetDateTime};

use crate::errors::*;

/// Get the current time relative to the Unix epoch, as suggested by the `time`
/// crate. (Why are we using the `time` crate? Could we do this using the
/// standard library or `chrono` instead?)
pub fn now() -> Duration {
    OffsetDateTime::now_utc() - OffsetDateTime::UNIX_EPOCH
}

/// Specifies an optional single-byte character used to configure our CSV
/// parser.
#[derive(Debug)]
pub struct CharSpecifier(Option<u8>);

impl CharSpecifier {
    /// Return the specified character, if any.
    pub fn char(&self) -> Option<u8> {
        self.0
    }
}

impl FromStr for CharSpecifier {
    type Err = Error;

    fn from_str(s: &str) -> Result<CharSpecifier> {
        if s.as_bytes().len() == 1 {
            Ok(CharSpecifier(Some(s.as_bytes()[0])))
        } else {
            match s {
                // For convenience so users to can type `"\t"` in most shells
                // instead of trying to type a tab literal. `xsv` supports this,
                // too.
                r"\t" => Ok(CharSpecifier(Some(b'\t'))),
                "tab" => Ok(CharSpecifier(Some(b'\t'))),
                "none" => Ok(CharSpecifier(None)),
                _ => Err(format_err!("cannot parse character specifier: '{}'", s)),
            }
        }
    }
}

#[test]
fn parses_char_specifiers() {
    assert_eq!(CharSpecifier::from_str(",").unwrap().char(), Some(b','));
    assert_eq!(CharSpecifier::from_str("\t").unwrap().char(), Some(b'\t'));
    assert_eq!(CharSpecifier::from_str(r"\t").unwrap().char(), Some(b'\t'));
    assert_eq!(CharSpecifier::from_str(r"tab").unwrap().char(), Some(b'\t'));
    assert_eq!(CharSpecifier::from_str(r"none").unwrap().char(), None);
}
