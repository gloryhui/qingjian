//! 一次固定音节切分的词图与逐位置剪枝记录。

use super::{PathDiagnostic, SpanDiagnostic, TransitionDiagnostic};

#[derive(Debug, Default)]
pub struct SearchDiagnostics {
    pub spans: Vec<SpanDiagnostic>,

    pub paths: Vec<PathDiagnostic>,

    pub transitions: Vec<TransitionDiagnostic>,
}
