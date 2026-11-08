use serde::Serialize;
/// One classified cell: an uppercase letter, or empty when the model saw no
/// readable glyph, with the classifier's softmax probability.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Recognition {
    pub text: String,
    pub confidence: f64,
}
