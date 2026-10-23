//! 分段选择事务：暂存候选、撤销选词，整段确认时才学习与上屏。

mod selected;

pub(super) use selected::Selected;

use crate::candidate::{Candidate, CandidateKind};
use crate::engine::{Engine, InputSource};

impl Engine {
    /// 选择候选；还有未选拼音时只更新组句，全部选完才返回一次完整上屏文本。
    pub fn select_candidate(&mut self, candidate: &Candidate) -> Option<String> {
        let consumed = match candidate.kind {
            CandidateKind::Chinese | CandidateKind::Sentence => self.consumed_by(candidate).0,
            CandidateKind::Emoji if !candidate.syllables.is_empty() => {
                self.consumed_by(candidate).0
            }
            _ => self.whole_scope().0,
        };
        let mut remaining = self.composition.clone();
        remaining.drain_prefix(consumed);
        if consumed == 0 || remaining.is_empty() {
            return Some(self.commit(candidate));
        }
        let input = self.composition.take_prefix(consumed);
        self.selected.push(Selected {
            candidate: candidate.clone(),
            input,
            query: self.last_query.borrow().clone(),
            aux_code: self.aux_code.take(),
        });
        self.forget_span_cache();
        None
    }

    /// 撤销最近一次分段选词，保留后来输入或修改过的尾段。
    pub fn undo_selection(&mut self) -> bool {
        let Some(selected) = self.selected.pop() else {
            return false;
        };
        self.composition.prepend(&selected.input);
        self.aux_code = selected.aux_code;
        self.forget_span_cache();
        true
    }

    pub fn is_composing(&self) -> bool {
        !self.selected.is_empty() || !self.composition.is_empty()
    }

    pub(super) fn selected_text(&self) -> String {
        self.selected
            .iter()
            .map(|s| s.candidate.text.as_str())
            .collect()
    }

    /// 按原顺序重放暂存选择，沿用现有学习、日志与自动造词路径，只在真正上屏时调用。
    pub(super) fn commit_selected(&mut self) -> String {
        let selected = std::mem::take(&mut self.selected);
        if self.composition.cursor() == 0 {
            self.composition.move_end();
        }
        for part in selected.iter().rev() {
            self.composition.prepend(&part.input);
        }
        let tail_aux = self.aux_code.take();
        let tail_query = self.last_query.borrow().clone();
        let mut text = String::new();
        for part in selected {
            self.aux_code = part.aux_code;
            *self.last_query.borrow_mut() = part.query;
            text.push_str(&self.commit_with(
                &part.candidate,
                InputSource::from(part.candidate.kind),
                None,
            ));
        }
        self.aux_code = tail_aux;
        *self.last_query.borrow_mut() = tail_query;
        text
    }
}
