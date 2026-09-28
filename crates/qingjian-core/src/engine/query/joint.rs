//! 多种拼音切分共享整句评分和神经重排，首选切分随首选候选返回。

use crate::candidate::{Candidate, CandidateKind};
use crate::engine::{Engine, Learner, RESCORE_PATHS, choice_key};
use crate::parser::Segmentation;
use crate::sentence::diversity::representative_indices;
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
            let path_groups = sentence::convert_path_groups(
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
            let found: Vec<_> = path_groups
                .into_iter()
                .flatten()
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
        ranked.sort_by(|a, b| {
            b.1.score
                .total_cmp(&a.1.score)
                .then_with(|| a.1.text.cmp(&b.1.text))
                .then_with(|| a.0.cmp(&b.0))
        });
        if ranked.is_empty() {
            return None;
        }
        if k == 1 {
            return Some(ranked.remove(0));
        }
        self.retain_neural_eligible_by_text(
            &mut ranked,
            |(_, path)| &path.text,
            |(_, path)| path.score,
        );
        let mut selected = select_joint_paths(&ranked, k);
        let mut paths: Vec<Conversion> = selected.iter().map(|(_, path)| path.clone()).collect();
        self.rescore_path_scores(&mut paths);
        for ((_, selected_path), rescored_path) in selected.iter_mut().zip(paths) {
            selected_path.score = rescored_path.score;
        }
        selected.sort_by(|left, right| right.1.score.total_cmp(&left.1.score));
        selected.into_iter().next()
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
                let ranked_full_word = items
                    .iter()
                    .find(|candidate| candidate.kind == CandidateKind::Chinese)
                    .is_some_and(|candidate| {
                        candidate.syllables.concat()
                            == choice_key(self.composition.scope(), self.composition.scope().len())
                    });
                let promote = !ranked_full_word
                    && index > 0
                    && segmentations[0].syllables.len() >= 2
                    && segmentations[index].syllables.iter().all(|p| p.complete);
                if promote {
                    let candidate = items.remove(position);
                    let target = leading_english(items);
                    items.insert(target, candidate);
                }
                align_first_chinese_segmentation(items, segmentations);
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
        let duplicate_is_first = !conversion.altered()
            && items.get(leading_english(items)).is_some_and(|first| {
                first.kind == CandidateKind::Chinese
                    && first.text == conversion.text
                    && first.syllables == conversion.syllables
            });
        let candidate = self.sentence_candidate(items, &segmentations[index], conversion);
        if candidate.is_some() || duplicate_is_first {
            segmentations.swap(0, index);
        }
        candidate
    }
}

fn align_first_chinese_segmentation(items: &[Candidate], segmentations: &mut [Segmentation]) {
    let Some(first) = items.get(leading_english(items)) else {
        return;
    };
    if first.kind != CandidateKind::Chinese {
        return;
    }
    if let Some(index) = segmentations.iter().position(|segmentation| {
        segmentation.syllables.len() == first.syllables.len()
            && segmentation
                .syllables
                .iter()
                .zip(&first.syllables)
                .all(|(syllable, reading)| syllable.text == *reading)
    }) {
        segmentations.swap(0, index);
    }
}

/// 两种切分轮流取各自词路径的分歧代表，再按总分填满剩余名额。
fn select_joint_paths(ranked: &[(usize, Conversion)], k: usize) -> Vec<(usize, Conversion)> {
    let mut text_groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, (_, path)) in ranked.iter().enumerate() {
        if let Some((_, indices)) = text_groups.iter_mut().find(|(text, _)| text == &path.text) {
            indices.push(index);
        } else {
            text_groups.push((path.text.clone(), vec![index]));
        }
    }
    let mut segmentation_groups: Vec<(usize, Vec<usize>, Vec<String>)> = Vec::new();
    for (segmentation, path) in ranked {
        let text_index = text_groups
            .iter()
            .position(|(text, _)| text == &path.text)
            .expect("every ranked path has a text group");
        let group = if let Some(group) = segmentation_groups
            .iter_mut()
            .find(|(seen, _, _)| seen == segmentation)
        {
            group
        } else {
            segmentation_groups.push((*segmentation, Vec::new(), Vec::new()));
            segmentation_groups.last_mut().expect("just inserted")
        };
        if !group.1.contains(&text_index) {
            group.1.push(text_index);
            group.2.push(
                path.words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\0"),
            );
        }
    }
    let representatives: Vec<Vec<usize>> = segmentation_groups
        .iter()
        .map(|(_, text_indices, routes)| {
            let refs: Vec<&str> = routes.iter().map(String::as_str).collect();
            representative_indices(&refs, k)
                .into_iter()
                .map(|index| text_indices[index])
                .collect()
        })
        .collect();
    let mut selected_groups = Vec::new();
    let mut round = 0;
    while selected_groups.len() < k && representatives.iter().any(|group| group.len() > round) {
        for group in &representatives {
            if let Some(&text_index) = group.get(round)
                && !selected_groups.contains(&text_index)
            {
                selected_groups.push(text_index);
                if selected_groups.len() == k {
                    break;
                }
            }
        }
        round += 1;
    }
    for text_index in 0..text_groups.len() {
        if selected_groups.len() >= k {
            break;
        }
        if !selected_groups.contains(&text_index) {
            selected_groups.push(text_index);
        }
    }
    let mut selected: Vec<(usize, Conversion)> = selected_groups
        .into_iter()
        .flat_map(|text_index| {
            text_groups[text_index]
                .1
                .iter()
                .map(|index| ranked[*index].clone())
        })
        .collect();
    selected.sort_by(|left, right| right.1.score.total_cmp(&left.1.score));
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentence::SentenceWord;
    use qingjian_dictionary::Dictionary;

    fn path(text: &str, first: &str, score: f64) -> Conversion {
        route(text, &[first, text], score, score, 0.0)
    }

    fn route(
        text: &str,
        words: &[&str],
        score: f64,
        static_score: f64,
        penalty: f64,
    ) -> Conversion {
        Conversion {
            text: text.to_owned(),
            syllables: Vec::new(),
            words: words
                .iter()
                .map(|word| SentenceWord {
                    text: (*word).to_owned(),
                    syllables: Vec::new(),
                    placeholder: false,
                })
                .collect(),
            score,
            static_score,
            penalty,
        }
    }

    #[test]
    fn global_budget_keeps_segmentation_and_structural_representatives() {
        let mut ranked = vec![
            (0, path("A1", "晚", -10.0)),
            (0, path("A2", "晚", -10.1)),
            (0, path("A3", "晚", -10.2)),
            (1, path("B1", "甲", -10.3)),
            (0, path("A4", "万", -10.4)),
            (0, path("A5", "万", -10.5)),
            (1, path("B2", "乙", -10.6)),
            (1, path("B3", "丙", -10.7)),
            (0, path("A_target", "玩", -10.9)),
        ];
        let selected = select_joint_paths(&ranked, 6);
        assert!(selected.iter().any(|(_, path)| path.text == "A_target"));
        assert!(selected.iter().any(|(index, _)| *index == 1));
        let again = select_joint_paths(&ranked, 6);
        assert_eq!(
            selected
                .iter()
                .map(|(_, path)| &path.text)
                .collect::<Vec<_>>(),
            again.iter().map(|(_, path)| &path.text).collect::<Vec<_>>()
        );
        ranked.push((0, path("A1", "晚", -10.95)));
        assert_eq!(
            select_joint_paths(&ranked, 6)
                .iter()
                .filter(|(_, path)| path.text == "A1")
                .count(),
            2
        );
    }

    struct ConstantScorer;

    impl crate::sentence::SentenceScorer for ConstantScorer {
        fn score(&self, _context: &str, texts: &[&str]) -> Vec<f64> {
            vec![-1.0; texts.len()]
        }
    }

    #[test]
    fn same_text_routes_survive_selection_order_and_keep_path_specific_scores() {
        let first = (0, route("同句", &["同", "句"], -2.0, -1.0, 0.0));
        let second = (0, route("同句", &["同句"], -2.2, -4.0, 0.5));
        let third = (1, route("同句", &["同", "句"], -2.3, -3.0, 0.0));
        let other = (0, route("别句", &["别", "句"], -3.0, -3.0, 0.0));

        for ranked in [
            vec![first.clone(), second.clone(), third.clone(), other.clone()],
            vec![second, first, third, other],
        ] {
            let selected = select_joint_paths(&ranked, 1);
            assert_eq!(selected.len(), 3);
            assert!(selected.iter().all(|(_, path)| path.text == "同句"));

            let engine = Engine::new(Dictionary::parse("同\ttong\t100\n").unwrap())
                .with_sentence_scorer(Box::new(ConstantScorer), Some(0.5), None, None);
            let mut paths: Vec<_> = selected.iter().map(|(_, path)| path.clone()).collect();
            engine.rescore_paths(&mut paths);
            assert_eq!(paths[0].words[0].text, "同句");
            assert!(
                (paths[0].score + 0.7).abs() < 1e-12,
                "selected path {:?} has score {}",
                paths[0].words,
                paths[0].score
            );
        }
    }
}
