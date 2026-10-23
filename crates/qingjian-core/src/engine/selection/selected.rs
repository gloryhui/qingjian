//! 一次尚未上屏的分段选择，保留原始按键供退格恢复。

use crate::candidate::Candidate;
use crate::composition::Composition;
use crate::engine::query::QuerySnapshot;

pub(in crate::engine) struct Selected {
    pub candidate: Candidate,

    pub input: Composition,

    pub query: Option<QuerySnapshot>,

    pub aux_code: Option<String>,
}
