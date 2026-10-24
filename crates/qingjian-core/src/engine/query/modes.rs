//! 表达式、英文、直输与问字模式的候选生成。

use super::Query;
use crate::candidate::{Candidate, CandidateKind, CandidateList};
use crate::engine::{ENGLISH_MODE_CANDIDATES, Engine, Learner, Timings};
use crate::{english, shortcut};
use std::time::{Duration, Instant};

impl Engine {
    /// 表达式模式（`v` 开头）：不解析拼音，候选是算式结果 / 中文数字，再加上整段是英文词的情况（`very`）。
    /// preedit 原样显示输入。
    pub(super) fn query_expression(&self, scope: &str, rest: String, start: Instant) -> Query {
        let mut items = shortcut::candidates(scope, self.modes().expression, &jiff::Zoned::now());
        if let Some(word) = self.english.as_ref().and_then(|english| english.get(scope)) {
            items.push(Candidate {
                text: word.to_owned(),
                kind: CandidateKind::English,
                syllables: Vec::new(),
                reading: None,
                translation: None,
                aux_code: None,
            });
        }
        Query {
            selected: String::new(),
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            tail: scope.to_owned(),
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            decoded_keys: self.shuangpin.is_some() || self.zhuyin,
            shuangpin_raw_preedit: false,
            typed_display: Some(self.composition.typed_scope()),
            correction: None,
            aux: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 英文直输段：唯一候选就是原文（`no-way`），空格 / 回车都上屏它；preedit 原样显示。
    pub(super) fn query_raw(&self, scope: &str, rest: String, start: Instant) -> Query {
        let items = vec![Candidate {
            text: self.composition.typed_scope(),
            kind: CandidateKind::English,
            syllables: Vec::new(),
            reading: None,
            translation: None,
            aux_code: None,
        }];
        Query {
            selected: String::new(),
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            tail: scope.to_owned(),
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            decoded_keys: self.shuangpin.is_some() || self.zhuyin,
            shuangpin_raw_preedit: false,
            typed_display: Some(self.composition.typed_scope()),
            correction: None,
            aux: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 英文模式：敲的字母原样显示，候选是英文词表的精确词、前缀补全与拼错纠正（见 [`english::suggest`]），
    /// 词表没装就没有候选。emoji 照配，但排在所有词后面：选词靠上下键，emoji 夹在词中间会挡路。
    pub(super) fn query_english(&self, scope: &str, rest: String, start: Instant) -> Query {
        let mut items: Vec<Candidate> = english::suggest(
            &self.english_lists(),
            scope,
            |text| self.learner.weight(text),
            ENGLISH_MODE_CANDIDATES,
        )
        .into_iter()
        .map(|text| Candidate {
            text,
            kind: CandidateKind::English,
            syllables: Vec::new(),
            reading: None,
            translation: None,
            aux_code: None,
        })
        .collect();
        self.insert_emoji(&mut items);
        items.sort_by_key(|c| c.kind == CandidateKind::Emoji);
        Query {
            selected: String::new(),
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            tail: scope.to_owned(),
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            decoded_keys: self.shuangpin.is_some() || self.zhuyin,
            shuangpin_raw_preedit: false,
            typed_display: None,
            correction: None,
            aux: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 问字模式（问字键或 `?` 开头）：拼音问题本地没有候选，preedit 显示前缀加切分好的问题拼音，答案等云端；
    /// 十六进制码点（`u4e00`、`u+1f600`）本地直接给出那个字符。
    pub(super) fn query_question(&self, scope: &str, rest: String, start: Instant) -> Query {
        let body = self.modes().question_body(scope, self.zhuyin);
        let typed = self.composition.typed_scope();
        let prefix_len = scope.len() - body.len();
        let prefix = &typed[..prefix_len];
        let (candidates, tail) = match shortcut::unicode_form(body) {
            Some(text) => (
                CandidateList {
                    items: vec![Candidate {
                        text,
                        kind: CandidateKind::Shortcut,
                        syllables: Vec::new(),
                        reading: None,
                        translation: None,
                        aux_code: None,
                    }],
                },
                typed.clone(),
            ),
            None => (
                CandidateList::default(),
                format!("{prefix}{}", self.marked_rest(&typed[prefix_len..])),
            ),
        };
        Query {
            selected: String::new(),
            segmentations: Vec::new(),
            candidates,
            tail,
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            decoded_keys: self.shuangpin.is_some() || self.zhuyin,
            shuangpin_raw_preedit: false,
            typed_display: None,
            correction: None,
            aux: None,
            timings: Timings {
                parse: start.elapsed(),
                lookup: Duration::ZERO,
                rank: Duration::ZERO,
            },
        }
    }
}
