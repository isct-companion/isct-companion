#[cfg(any(matrix_ocr, test))]
use crate::MatrixOcrError;
use serde::Serialize;
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Recognition {
    pub text: String,
    pub confidence: f64,
}
#[cfg(any(matrix_ocr, test))]
fn letter(text: &str) -> &str {
    let raw = text.trim();
    match raw {
        "0" => "O",
        "1" => "I",
        "8" => "B",
        _ if raw.len() == 1 && raw.as_bytes()[0].is_ascii_uppercase() => raw,
        _ => "",
    }
}
#[cfg(any(matrix_ocr, test))]
fn converted(text: &str) -> bool {
    matches!(text.trim(), "0" | "1" | "8")
}
#[cfg(any(matrix_ocr, test))]
pub(crate) fn accept(text: &str, confidence: f64) -> Recognition {
    let value = letter(text);
    if value.is_empty() || !confidence.is_finite() || !(0.65..=1.).contains(&confidence) {
        return Recognition::default();
    }
    Recognition {
        text: value.into(),
        confidence: if converted(text) {
            confidence.min(0.89)
        } else {
            confidence
        },
    }
}
#[cfg(any(matrix_ocr, test))]
pub(crate) fn review(mut cleaned: Recognition, original: &str) -> Recognition {
    if !cleaned.text.is_empty() {
        if cleaned.text != letter(original) {
            cleaned.confidence = cleaned.confidence.min(0.74);
        } else if converted(original) {
            cleaned.confidence = cleaned.confidence.min(0.89);
        }
    }
    cleaned
}
#[cfg(any(all(matrix_ocr, debug_assertions), test))]
pub(crate) fn is_red(result: &Recognition) -> bool {
    result.text.is_empty() || result.confidence < 0.75
}
#[cfg(any(matrix_ocr, test))]
pub(crate) fn decode(data: &[f32], characters: &[String]) -> Result<Recognition, MatrixOcrError> {
    if characters.is_empty() || !data.len().is_multiple_of(characters.len()) {
        return Err(MatrixOcrError::Model(
            "Unexpected Paddle output shape".into(),
        ));
    }
    let (mut previous, mut text, mut confidence, mut count) = (usize::MAX, String::new(), 0., 0);
    for step in data.chunks_exact(characters.len()) {
        let (mut best, mut score) = (0, f32::NEG_INFINITY);
        for (i, &p) in step.iter().enumerate() {
            if p > score {
                best = i;
                score = p;
            }
        }
        if best != 0 && best != previous {
            text.push_str(&characters[best]);
            confidence += score as f64;
            count += 1;
        }
        previous = best;
    }
    Ok(Recognition {
        text,
        confidence: if count > 0 {
            confidence / count as f64
        } else {
            0.
        },
    })
}

#[cfg(test)]
#[path = "tests/recognition.rs"]
mod tests;
