use super::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService};
use serde::{Deserialize, Serialize};

impl SchoolAuthService {
    pub fn matrix_text(&self) -> Result<String, SchoolAuthError> {
        let credentials = self.load(SchoolAuthProfile::TokyoTech)?.ok_or_else(|| {
            SchoolAuthError::invalid("Save the Tokyo Tech Matrix before copying it.")
        })?;
        matrix_plain_text(&credentials.matrix)
    }
}

const MATRIX_ROWS: usize = 7;
const MATRIX_COLUMNS: usize = 10;

/// A paste-friendly rendering in the same row-major order as the editor:
/// A1…J1, A2…J2, …, A7…J7.
fn matrix_plain_text(matrix: &str) -> Result<String, SchoolAuthError> {
    let cells: Vec<char> = matrix.chars().collect();
    if cells.len() != MATRIX_ROWS * MATRIX_COLUMNS {
        return Err(SchoolAuthError::invalid(
            "Matrix must contain 7 rows × 10 columns.",
        ));
    }
    Ok(cells
        .chunks(MATRIX_COLUMNS)
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The order a pasted Matrix lists its cells in. Ours (and the copy above)
/// goes along each row, A1…J1, A2…; titech-app's export goes down
/// each column, A1…A7, B1….
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatrixOrder {
    Rows,
    Columns,
}

/// A pasted Matrix: the 70 cells in row order, and the order the paste
/// listed them in (`Columns` means it was titech-app's export).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixPaste {
    pub cells: String,
    pub order: MatrixOrder,
}

/// Reads pasted Matrix text into the 70 cells in row order (A1…J1, A2…).
///
/// Two shapes are accepted: letters along rows with any whitespace between
/// them (our own copy), or titech-app's export, a JSON array of 70
/// one-letter strings listed down columns. Anything else is rejected rather
/// than silently shifted or truncated.
pub fn parse_matrix_text(text: &str) -> Result<MatrixPaste, SchoolAuthError> {
    let invalid = || {
        SchoolAuthError::invalid("Paste exactly 70 letters; spaces and line breaks are ignored.")
    };
    let (letters, order): (Vec<char>, MatrixOrder) =
        match serde_json::from_str::<Vec<String>>(text.trim()) {
            Ok(entries) => (
                entries
                    .iter()
                    .map(|entry| {
                        let mut characters = entry.trim().chars();
                        match (characters.next(), characters.next()) {
                            (Some(letter), None) => Ok(letter),
                            _ => Err(invalid()),
                        }
                    })
                    .collect::<Result<_, _>>()?,
                MatrixOrder::Columns,
            ),
            Err(_) => (
                text.chars().filter(|c| !c.is_whitespace()).collect(),
                MatrixOrder::Rows,
            ),
        };
    let letters: Vec<char> = letters.iter().map(char::to_ascii_uppercase).collect();
    if letters.len() != MATRIX_ROWS * MATRIX_COLUMNS
        || !letters.iter().all(char::is_ascii_uppercase)
    {
        return Err(invalid());
    }
    let cells = match order {
        MatrixOrder::Rows => letters.into_iter().collect(),
        MatrixOrder::Columns => (0..MATRIX_ROWS * MATRIX_COLUMNS)
            .map(|cell| {
                let (row, column) = (cell / MATRIX_COLUMNS, cell % MATRIX_COLUMNS);
                letters[column * MATRIX_ROWS + row]
            })
            .collect(),
    };
    Ok(MatrixPaste { cells, order })
}

#[cfg(test)]
#[path = "../tests/school_auth__matrix.rs"]
mod tests;
