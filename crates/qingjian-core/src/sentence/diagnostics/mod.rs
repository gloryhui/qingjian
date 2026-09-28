//! 整句词图与束搜索的按需诊断；普通查询不创建这些记录。

mod path;
mod search;
mod span;
mod transition;

pub use path::PathDiagnostic;
pub use search::SearchDiagnostics;
pub use span::SpanDiagnostic;
pub use transition::TransitionDiagnostic;
