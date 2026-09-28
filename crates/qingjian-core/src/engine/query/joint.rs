//! 多种拼音切分共享整句评分和神经重排，首选切分随首选候选返回。

use crate::candidate::{Candidate, CandidateKind};
use crate::engine::{Engine, Learner, RESCORE_PATHS};
use crate::parser::Segmentation;
use crate::sentence::{self, Conversion};

use super::leading_english;

/// 每键整句最多搜索两种音节切分；其余切分仍参与词级查询。
const JOINT_SEGMENTATIONS: usize = 2;

impl Engine {
    /// 词格与静态/个人路径在每种切分上分别搜索，再把可比的完整路径放在一起重排。
    pub(super) fn best_joint_sentence(
        &self,
        segmentations: &[Segmentation],
        items: &[Candidate],
        typos: bool,
    ) -> Option<(usize, Conversion)> {
        let complete_exists = segmentations
            .iter()
            .any(|s| s.syllables.iter().all(|p| p.complete));
        let k = if self.has_sentence_scorer() {
            RESCORE_PATHS
        } else {
            1
        };
        let dictionaries = self.all_dictionaries();
        let mut ranked = Vec::new();
        let mut order: Vec<usize> = segmentations
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.syllables.len() >= 2
                    && (!complete_exists || s.syllables.iter().all(|p| p.complete))
            })
            .map(|(index, _)| index)
            .collect();
        // 词级查询已遍历全部切分。若较后的切分有整段精确词，优先让它
        // 与 parser 的首选进入有限的整句预算，而不是再按最长音节排序选第二条。
        if let Some(position) = order.iter().skip(1).position(|index| {
            let segmentation = &segmentations[*index];
            items.iter().any(|candidate| {
                candidate.kind == CandidateKind::Chinese
                    && candidate.syllables.len() == segmentation.syllables.len()
                    && candidate
                        .syllables
                        .iter()
                        .zip(&segmentation.syllables)
                        .all(|(reading, syllable)| reading == &syllable.text)
            })
        }) {
            order.swap(1, position + 1);
        }
        for index in order.into_iter().take(JOINT_SEGMENTATIONS) {
            let segmentation = &segmentations[index];
            let expanded = self.expand_positions(&segmentation.patterns(), typos);
            let paths = sentence::convert_paths(
                &dictionaries,
                &expanded.positions(),
                false,
                k,
                &*self.language_model,
                self.personal(),
                |text| self.learner.weight(text),
                |position, syllable| expanded.cost(position, syllable),
                &mut self.span_cache.borrow_mut(),
            );
            let found: Vec<_> = paths
                .into_iter()
                .filter_map(|path| {
                    let trailing_partial = segmentation
                        .syllables
                        .last()
                        .is_some_and(|last| !last.complete && last.text.len() == 1);
                    let covered = path.syllables.len() == segmentation.syllables.len()
                        || (trailing_partial
                            && path.syllables.len() + 1 == segmentation.syllables.len());
                    (!path.has_placeholder() && covered).then_some((index, path))
                })
                .collect();
            ranked.extend(found);
        }
        ranked.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
        if ranked.is_empty() {
            return None;
        }
        if k == 1 {
            return Some(ranked.remove(0));
        }
        // 半数名额给不同拼音切分的最佳路径，其余按总分补齐；文本去重避免一条汉字句占多席。
        let mut selected: Vec<(usize, Conversion)> = Vec::new();
        for (index, path) in &ranked {
            if selected.len() >= k / 2 {
                break;
            }
            if !selected.iter().any(|(seen, _)| seen == index)
                && !selected.iter().any(|(_, seen)| seen.text == path.text)
            {
                selected.push((*index, path.clone()));
            }
        }
        for (index, path) in ranked {
            if selected.len() >= k {
                break;
            }
            if !selected.iter().any(|(_, seen)| seen.text == path.text) {
                selected.push((index, path));
            }
        }
        let mut paths: Vec<Conversion> = selected.iter().map(|(_, path)| path.clone()).collect();
        self.rescore_paths(&mut paths);
        let best = paths.into_iter().next()?;
        let index = selected.iter().find(|(_, path)| path.text == best.text)?.0;
        Some((index, best))
    }

    /// 整句胜出的切分成为 preedit 的首选切分；完整词由已有词级候选承载。
    pub(super) fn plain_sentence_joint(
        &self,
        items: &mut Vec<Candidate>,
        segmentations: &mut [Segmentation],
        typos: bool,
    ) -> Option<Candidate> {
        if segmentations.first()?.syllables.len() < 2 {
            return None;
        }
        let (index, conversion) = self.best_joint_sentence(segmentations, items, typos)?;
        if conversion.word_count() == 1 && !conversion.altered() {
            // 单音节输入的词级排序已有独立依据；只在多音节切分歧义确实被
            // 另一条完整切分的词汇证据推翻时提升词候选。
            if let Some(position) = items.iter().position(|candidate| {
                candidate.kind == CandidateKind::Chinese
                    && candidate.text == conversion.text
                    && candidate.syllables == conversion.syllables
            }) {
                let promote = index > 0
                    && segmentations[0].syllables.len() >= 2
                    && segmentations[index].syllables.iter().all(|p| p.complete);
                if promote {
                    let candidate = items.remove(position);
                    let target = leading_english(items);
                    items.insert(target, candidate);
                }
                if promote || items.first().is_some_and(|c| c.text == conversion.text) {
                    segmentations.swap(0, index);
                }
            } else if index > 0
                && conversion.syllables.len() == segmentations[index].syllables.len()
            {
                segmentations.swap(0, index);
                return Some(Candidate {
                    text: conversion.text,
                    kind: CandidateKind::Chinese,
                    syllables: conversion.syllables,
                    reading: None,
                    translation: None,
                    aux_code: None,
                });
            }
            return None;
        }
        let candidate = self.sentence_candidate(items, &segmentations[index], conversion);
        if candidate.is_some() {
            segmentations.swap(0, index);
        }
        candidate
    }
}
