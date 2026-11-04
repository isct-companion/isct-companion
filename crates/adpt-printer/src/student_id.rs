//! The student ID the printer bills a job to.

use std::fmt;

/// A student ID: two digits of the entry year, the programme letter (B
/// bachelor, M master, D doctor, R research student) and five digits, e.g.
/// `24B12345`. The printer bills whatever ID the LPD user field carries and
/// checks nothing, so this is the one gate on what reaches that field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StudentId(String);

impl StudentId {
    /// The ID in `text`, in capitals; `None` unless `text` is exactly one.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let shaped = bytes.len() == 8
            && matches!(bytes[2].to_ascii_uppercase(), b'B' | b'M' | b'D' | b'R')
            && bytes[..2].iter().chain(&bytes[3..]).all(u8::is_ascii_digit);
        shaped.then(|| Self(text.to_ascii_uppercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StudentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
