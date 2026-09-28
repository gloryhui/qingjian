//! 一个束节点在剪枝前的完整路径及其分数来源。

#[derive(Debug, Clone)]
pub struct PathDiagnostic {
    pub position: usize,

    pub text: String,

    pub words: Vec<String>,

    pub score: f64,

    pub static_score: f64,

    pub fallback_score: f64,

    pub personal_delta: f64,

    pub selection_bonus: f64,

    pub penalty: f64,

    pub retained: bool,
}
