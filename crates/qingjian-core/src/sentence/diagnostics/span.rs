//! 单个音节跨度的候选词记录。

/// 一个词图格子的候选（截断后）。
#[derive(Debug, Clone)]
pub struct SpanDiagnostic {
    pub start: usize,

    pub end: usize,

    pub words: Vec<String>,
}
