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
