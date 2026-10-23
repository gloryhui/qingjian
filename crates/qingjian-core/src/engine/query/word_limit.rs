//! 词语候选的显示范围：前段保留完整排序，后段只保留单字。

use crate::candidate::{Candidate, CandidateKind};
use crate::engine::Engine;

impl Engine {
    /// 词语与整句只占前若干个候选位置，之后保留单字；None 保留原有列表。
    pub fn set_word_candidate_limit(&mut self, limit: Option<usize>) {
        self.word_candidate_limit = limit;
    }

    pub(super) fn limit_word_candidates(&self, items: &mut Vec<Candidate>) {
        if !self.english_mode
            && let Some(limit) = self.word_candidate_limit
        {
            let mut position = 0;
            items.retain(|candidate| {
                let single = matches!(candidate.kind, CandidateKind::Chinese | CandidateKind::Code)
                    && candidate.text.chars().count() == 1;
                let keep = position < limit || single;
                position += 1;
                keep
            });
        }
    }
}
