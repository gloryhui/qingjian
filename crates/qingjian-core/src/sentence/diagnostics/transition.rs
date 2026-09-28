//! 同一词格的不同前驱在只选最佳前驱时的竞争。

#[derive(Debug, Clone)]
pub struct TransitionDiagnostic {
    pub end: usize,

    pub previous_text: String,

    pub word: String,

    pub score: f64,

    pub selected: bool,
}
