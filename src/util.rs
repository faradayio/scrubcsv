//! Miscellaneous utilities.

use errors::*;

/// Parse a character specifier and return a single-byte character.
pub fn parse_char_specifier(specifier: &str) -> Result<Option<u8>> {
    if specifier.as_bytes().len() == 1 {
        Ok(Some(specifier.as_bytes()[0]))
    } else {
        match specifier {
            // For convenience so users to can type `"\t"` in most shells
            // instead of trying to type a tab literal. `xsv` supports this,
            // too.
            r"\t" => Ok(Some('\t' as u8)),
            "none" => Ok(None),
            _ => {
                Err(ErrorKind::CannotParseCharacter(specifier.to_owned()).into())
            }
        }
    }
}

#[test]
fn parses_char_specifiers() {
    assert_eq!(parse_char_specifier(",").unwrap(), Some(',' as u8));
    assert_eq!(parse_char_specifier("\t").unwrap(), Some('\t' as u8));
    assert_eq!(parse_char_specifier(r"\t").unwrap(), Some('\t' as u8));
    assert_eq!(parse_char_specifier(r"none").unwrap(), None);
}
