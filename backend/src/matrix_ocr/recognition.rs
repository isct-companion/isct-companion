use serde::Serialize;
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Recognition {
    pub text: String,
    pub confidence: f64,
}
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
fn converted(text: &str) -> bool {
    matches!(text.trim(), "0" | "1" | "8")
}
pub fn accept(text: &str, confidence: f64) -> Recognition {
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
pub fn review(mut cleaned: Recognition, original: &str) -> Recognition {
    if !cleaned.text.is_empty() {
        if cleaned.text != letter(original) {
            cleaned.confidence = cleaned.confidence.min(0.74);
        } else if converted(original) {
            cleaned.confidence = cleaned.confidence.min(0.89);
        }
    }
    cleaned
}
#[cfg(any(debug_assertions, test))]
pub fn is_red(result: &Recognition) -> bool {
    result.text.is_empty() || result.confidence < 0.75
}
pub fn decode(data: &[f32], characters: &[String]) -> Result<Recognition, String> {
    if characters.is_empty() || !data.len().is_multiple_of(characters.len()) {
        return Err("Unexpected Paddle output shape".into());
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
mod tests {
    use super::*;
    #[test]
    fn conversions_and_rejection() {
        for (digit, text) in [("0", "O"), ("1", "I"), ("8", "B")] {
            assert_eq!(
                accept(digit, 0.99),
                Recognition {
                    text: text.into(),
                    confidence: 0.89
                }
            );
            assert_eq!(accept(digit, 0.7).confidence, 0.7);
            assert_eq!(review(accept(text, 0.99), digit).confidence, 0.89);
            assert_eq!(review(accept(digit, 0.99), text).confidence, 0.89);
        }
        for text in ["2", "5", "18", "A8", "OO", "l", ""] {
            assert_eq!(accept(text, 0.99), Recognition::default());
        }
        for score in [0.4, f64::NAN, 95.] {
            assert_eq!(accept("A", score), Recognition::default());
        }
        assert_eq!(review(accept("U", 0.98), "O").confidence, 0.74);
        assert!(is_red(&review(accept("Z", 0.9), "")));
    }
    #[test]
    fn ctc_keeps_repeats_separated_by_blank() {
        let chars = vec!["blank".into(), "A".into(), "B".into()];
        let data = [
            0., 0.9, 0.1, 0., 0.8, 0.2, 1., 0., 0., 0., 0.7, 0.3, 0., 0.1, 0.9,
        ];
        let result = decode(&data, &chars).unwrap();
        assert_eq!(result.text, "AAB");
        assert!((result.confidence - 2.5 / 3.).abs() < 1e-6);
    }
}
