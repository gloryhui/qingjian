//! 多种拼音切分共享整句评分和神经重排，首选切分随首选候选返回。
//!
//! # 为什么不是「固定前 N 条切分」
//!
//! 旧实现只让 parser 排序靠前的固定两条切分进入整句路径搜索，其余切分哪怕词格和语言模型
//! 证据明显更强，也没有机会参与竞争——「正确的解释排在第三条」就是死路。
//!
//! 现在分两步，预算跟着证据走：
//!
//! 1. **探针**：每条有资格的切分先跑一次「单前驱、不带路线串」的窄搜索，拿到它自己的最优路径分。
//!    这一步复用同一张词格缓存，后面真要展开时不会重复查词库。探针总量由
//!    [`JOINT_PROBE_SPANS`] 兜住，长输入不会因为切分多而线性变慢。
//! 2. **按证据分配展开名额**：探针分最高的切分一定展开，parser 首选与「音节数不同的最佳探针」
//!    作为结构代表保底，其余按探针分从高到低填，直到落后最优探针超过 [`JOINT_PROBE_SLACK`]
//!    或碰到硬上限 [`JOINT_MAX_SEGMENTATIONS`]。
//!
//! 所以决定生死的是「这条切分自己的词格 + 语言模型能给出多好的完整路径」，不是它在 parser
//! 输出里的名次；把常数从 2 改成 4 只是顺带的结果，换掉的是选择依据。

use qingjian_dictionary::Dictionary;

use crate::candidate::{Candidate, CandidateKind};
use crate::engine::{Engine, JointStats, Learner, RESCORE_PATHS};
use crate::fuzzy::Expanded;
use crate::parser::Segmentation;
use crate::sentence::diversity::representative_indices;
use crate::sentence::{self, Conversion};

use super::composed::composed_insert_position;
use super::leading_english;

/// 联合搜索一次最多展开几条切分。硬上限：探针再多也不会让热路径随输入长度线性膨胀。
const JOINT_MAX_SEGMENTATIONS: usize = 4;

/// 保底展开条数：结构代表不因预算被砍光。
const JOINT_SEGMENTATIONS_FLOOR: usize = 2;

/// 探针分落后最优探针超过这么多 nat 的切分不再展开：它的宽束搜索要翻盘得补回这么多分。
const JOINT_PROBE_SLACK: f64 = 6.0;

/// 一次查询里探针最多摊开多少个词格。四个音节左右的短输入每条切分才十来个格子，
/// 四条有资格切分都能覆盖到；九音节以上的长句按这个上限收敛到两条，探针代价不随切分条数线性膨胀。
/// 词格本来就要为真正展开的切分算，探针只是让它们提前算，代价是「多探的那几条」。
const JOINT_PROBE_SPANS: usize = 128;

/// 一次联合搜索的结果。
pub(super) struct JointOutcome {
    /// 获胜切分在 `segmentations` 里的下标与它的整句路径。
    pub winner: (usize, Conversion),

    /// 从词格里提取的未登录组合候选，已按门槛过滤、限量、排好序。
    pub generated: Vec<Conversion>,
}

impl Engine {
    /// 词格与静态/个人路径在选中的切分上分别搜索，再把可比的完整路径放在一起重排。
    pub(super) fn best_joint_sentence(
        &self,
        segmentations: &[Segmentation],
        items: &[Candidate],
        typos: bool,
    ) -> Option<JointOutcome> {
        let complete_exists = segmentations
            .iter()
            .any(|s| s.syllables.iter().all(|p| p.complete));
        let eligible: Vec<usize> = segmentations
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.syllables.len() >= 2
                    && (!complete_exists || s.syllables.iter().all(|p| p.complete))
            })
            .map(|(index, _)| index)
            .collect();
        if eligible.is_empty() {
            return None;
        }
        // 没有神经重排时只要一条路径：未登录组合候选走自己那条词格通道，不靠路径池里多留几条。
        // 多留会让 `diverse_predecessors` 与路线串在每一次按键上都全量跑一遍。
        let k = if self.has_sentence_scorer() {
            RESCORE_PATHS
        } else {
            1
        };
        let dictionaries = self.all_dictionaries();

        let spans_before = self.span_cache.borrow().len();
        let probes = self.probe_segmentations(segmentations, &eligible, &dictionaries, typos);
        let plan = plan_joint_segmentations(&probes, segmentations);
        let mut ranked = Vec::new();
        let mut search: Vec<(usize, Expanded)> = Vec::with_capacity(plan.len());
        for position in plan {
            let (index, probe) = &probes[position];
            let (index, segmentation) = (*index, &segmentations[*index]);
            let expanded = self.expand_positions(&segmentation.patterns(), typos);
            if k == 1 {
                // 探针跑的就是同一条窄搜索，结果直接复用，不再跑第二遍
                ranked.push((index, probe.clone()));
            } else {
                let groups = sentence::convert_path_groups(
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
                for path in groups.into_iter().flatten() {
                    if covers(&path, segmentation) {
                        ranked.push((index, path));
                    }
                }
            }
            search.push((index, expanded));
        }
        let stats = JointStats {
            parser_segmentations: segmentations.len(),
            eligible_segmentations: eligible.len(),
            probed_segmentations: probes.len(),
            searched_segmentations: search.len(),
            spans: self.span_cache.borrow().len().saturating_sub(spans_before),
            viterbi_paths: ranked.len(),
            generated_candidates: 0,
        };
        self.joint_stats.set(stats);
        if ranked.is_empty() {
            return None;
        }
        let typed = segmentations[0].joined("");
        // 组合候选的门槛按**重排前**的路径分比：它自己的分也是重排前的，
        // 拿重排后的最优分当参照会把两者放在不同尺度上。
        let best_score = ranked
            .iter()
            .map(|(_, path)| path.score)
            .max_by(f64::total_cmp)
            .expect("nonempty path pool");
        ranked.sort_by(|a, b| {
            b.1.score
                .total_cmp(&a.1.score)
                .then_with(|| a.1.text.cmp(&b.1.text))
                .then_with(|| a.0.cmp(&b.0))
        });
        let winner = if k == 1 {
            ranked.remove(0)
        } else {
            self.retain_neural_eligible_by_text(
                &mut ranked,
                |(_, path)| &path.text,
                |(_, path)| path.score,
            );
            let mut selected = select_joint_paths(&ranked, k);
            let mut paths: Vec<Conversion> =
                selected.iter().map(|(_, path)| path.clone()).collect();
            self.rescore_path_scores(&mut paths);
            for ((_, selected_path), rescored_path) in selected.iter_mut().zip(paths) {
                selected_path.score = rescored_path.score;
            }
            selected.sort_by(|left, right| right.1.score.total_cmp(&left.1.score));
            selected.into_iter().next().expect("nonempty selection")
        };
        let generated = self.composed_candidates(&search, segmentations, items, &typed, best_score);
        self.joint_stats.set(JointStats {
            generated_candidates: generated.len(),
            ..self.joint_stats.get()
        });
        Some(JointOutcome { winner, generated })
    }

    /// 每条有资格的切分先跑一次窄搜索（单前驱、无路线串）拿最优路径，作为它进入完整搜索的证据。
    /// 探针用的词格会进同一个缓存，选中的切分随后做宽搜索时不重复查词库；只用一条路径时
    /// （没有神经重排）探针结果就是最终结果，不再跑第二遍。
    fn probe_segmentations(
        &self,
        segmentations: &[Segmentation],
        eligible: &[usize],
        dictionaries: &[&Dictionary],
        typos: bool,
    ) -> Vec<(usize, Conversion)> {
        let mut probes = Vec::with_capacity(eligible.len());
        let mut budget = JOINT_PROBE_SPANS;
        for index in eligible {
            let segmentation = &segmentations[*index];
            let cost = lattice_spans(segmentation.syllables.len());
            if !probes.is_empty() && cost > budget {
                break;
            }
            budget = budget.saturating_sub(cost);
            let expanded = self.expand_positions(&segmentation.patterns(), typos);
            let groups = sentence::convert_path_groups(
                dictionaries,
                &expanded.positions(),
                false,
                1,
                &*self.language_model,
                self.personal(),
                |text| self.learner.weight(text),
                |position, syllable| expanded.cost(position, syllable),
                &mut self.span_cache.borrow_mut(),
            );
            // 组内第一条就是这条路线的最高分路径（节点已按分数降序）
            let Some(path) = groups
                .into_iter()
                .next()
                .and_then(|group| group.into_iter().next())
            else {
                continue;
            };
            if covers(&path, segmentation) {
                probes.push((*index, path));
            }
        }
        probes
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
        let typed = segmentations.first()?.joined("");
        let outcome = self.best_joint_sentence(segmentations, items, typos)?;
        let candidate = self.apply_joint_winner(items, segmentations, outcome.winner, &typed);
        // 组合候选在整句首选与获胜切分都定下来之后再插：它不该反过来影响
        // `should_prefer_complete_word` 或整句候选的去重判断。
        //
        // 胜出路径真的变成候选时，同文本的组合候选是重复项，去掉；没有变成候选
        // （被完整词挡下来、或路径本身不够格当整句）时留着，否则用户第一次就选不到它。
        let mut generated = outcome.generated;
        if let Some(candidate) = &candidate {
            generated.retain(|conversion| conversion.text != candidate.text);
        }
        self.insert_composed(items, &generated, &typed);
        candidate
    }

    /// 整句胜出路径落到候选表上：完整词由词级排序承载，多词路径作为整句候选。
    fn apply_joint_winner(
        &self,
        items: &mut Vec<Candidate>,
        segmentations: &mut [Segmentation],
        (index, conversion): (usize, Conversion),
        typed: &str,
    ) -> Option<Candidate> {
        if self.should_prefer_complete_word(items, &conversion, typed) {
            align_first_chinese_segmentation(items, segmentations);
            return None;
        }
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
                    .is_some_and(|candidate| candidate.syllables.concat() == typed);
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

    /// 未登录组合候选插在候选表里：开头英文候选之后、整句首选之后、覆盖整段输入的完整词之后。
    fn insert_composed(&self, items: &mut Vec<Candidate>, generated: &[Conversion], typed: &str) {
        if generated.is_empty() {
            return;
        }
        let mut position = composed_insert_position(items, typed);
        for conversion in generated {
            if items.iter().any(|item| item.text == conversion.text) {
                continue;
            }
            items.insert(
                position.min(items.len()),
                Candidate {
                    text: conversion.text.clone(),
                    kind: CandidateKind::Chinese,
                    syllables: conversion.syllables.clone(),
                    reading: None,
                    translation: None,
                    aux_code: None,
                },
            );
            position += 1;
        }
    }
}

/// 一条整句路径是否可用：不是占位音节，且覆盖了这条切分的全部音节。
fn covers(path: &Conversion, segmentation: &Segmentation) -> bool {
    if path.has_placeholder() {
        return false;
    }
    let trailing_partial = segmentation
        .syllables
        .last()
        .is_some_and(|last| !last.complete && last.text.len() == 1);
    path.syllables.len() == segmentation.syllables.len()
        || (trailing_partial && path.syllables.len() + 1 == segmentation.syllables.len())
}

/// 一条切分在词图上最多个跨度：每个起点最多 [`sentence::MAX_WORD_SYLLABLES`] 个长度。探针预算按它算。
fn lattice_spans(syllables: usize) -> usize {
    (0..syllables)
        .map(|start| (syllables - start).min(sentence::MAX_WORD_SYLLABLES))
        .sum()
}

/// 按探针分挑出真正要做多路径搜索的切分，返回 `probes` 里的下标（已按探针分降序）。
///
/// 保底三个代表先入：探针分最高的切分（证据最强）、parser 的首选（与旧行为连续）、
/// 以及音节数不同于最优探针的最佳切分（切分结构代表，避免名额全被同一类切分占掉）。
/// 其余按探针分从高到低填，落后最优探针超过 [`JOINT_PROBE_SLACK`] 就停。
fn plan_joint_segmentations(
    probes: &[(usize, Conversion)],
    segmentations: &[Segmentation],
) -> Vec<usize> {
    if probes.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..probes.len()).collect();
    order.sort_by(|left, right| {
        probes[*right]
            .1
            .score
            .total_cmp(&probes[*left].1.score)
            .then_with(|| probes[*left].0.cmp(&probes[*right].0))
    });
    let mut chosen: Vec<usize> = Vec::with_capacity(JOINT_MAX_SEGMENTATIONS);
    let take = |position: usize, chosen: &mut Vec<usize>| {
        if chosen.len() < JOINT_MAX_SEGMENTATIONS && !chosen.contains(&position) {
            chosen.push(position);
        }
    };
    take(order[0], &mut chosen);
    if let Some(position) = probes.iter().position(|(index, _)| *index == 0) {
        take(position, &mut chosen);
    }
    let best_syllables = segmentations[probes[order[0]].0].syllables.len();
    if let Some(&position) = order
        .iter()
        .find(|position| segmentations[probes[**position].0].syllables.len() != best_syllables)
    {
        take(position, &mut chosen);
    }
    for &position in &order {
        if chosen.len() >= JOINT_SEGMENTATIONS_FLOOR {
            break;
        }
        take(position, &mut chosen);
    }
    let floor = probes[order[0]].1.score - JOINT_PROBE_SLACK;
    for &position in &order {
        if chosen.len() >= JOINT_MAX_SEGMENTATIONS || probes[position].1.score < floor {
            break;
        }
        take(position, &mut chosen);
    }
    chosen.truncate(JOINT_MAX_SEGMENTATIONS);
    chosen
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

/// 各切分轮流取各自词路径的分歧代表，再按总分填满剩余名额。
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
    use crate::parser::Syllable;
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
            personal_bonus: 0.0,
            penalty,
        }
    }

    fn probe_scores(scores: &[(usize, f64)]) -> Vec<(usize, Conversion)> {
        scores
            .iter()
            .map(|(index, score)| (*index, route("", &[""], *score, *score, 0.0)))
            .collect()
    }

    fn segmentation(syllables: &[&str]) -> Segmentation {
        Segmentation {
            syllables: syllables
                .iter()
                .map(|text| Syllable::complete(text))
                .collect(),
        }
    }

    #[test]
    fn plan_keeps_the_strongest_probe_over_parser_order() {
        // parser 的前两条证据都很弱，第三条（下标 2）才是有词汇支撑的那条
        let segmentations = vec![
            segmentation(&["an", "xian", "an"]),
            segmentation(&["an", "xia", "nan"]),
            segmentation(&["an", "xi", "an", "an"]),
        ];
        let probes = probe_scores(&[(0, -19.0), (1, -17.0), (2, -2.0)]);
        let plan = plan_joint_segmentations(&probes, &segmentations);
        assert_eq!(probes[plan[0]].0, 2, "探针分最高的切分先展开：{plan:?}");
        assert!(
            plan.iter().any(|position| probes[*position].0 == 0),
            "parser 首选保底：{plan:?}"
        );
        assert!(plan.len() <= JOINT_MAX_SEGMENTATIONS);
    }

    #[test]
    fn plan_stops_expanding_once_the_probe_gap_is_too_wide() {
        let segmentations = vec![
            segmentation(&["wo", "men"]),
            segmentation(&["wo", "men", "de"]),
            segmentation(&["w", "o", "men"]),
            segmentation(&["wo", "m", "en"]),
        ];
        let probes = probe_scores(&[(0, -3.0), (1, -3.5), (2, -40.0), (3, -41.0)]);
        let plan = plan_joint_segmentations(&probes, &segmentations);
        let indices: Vec<usize> = plan.iter().map(|position| probes[*position].0).collect();
        assert!(!indices.contains(&2), "落后太多不展开：{plan:?}");
        assert!(!indices.contains(&3), "落后太多不展开：{plan:?}");
    }

    #[test]
    fn plan_keeps_a_different_syllable_count_as_structure_representative() {
        let segmentations = vec![
            segmentation(&["ken", "eng"]),
            segmentation(&["ke", "neng"]),
            segmentation(&["ke", "n", "eng"]),
        ];
        let probes = probe_scores(&[(0, -9.0), (1, -8.0), (2, -9.2)]);
        let plan = plan_joint_segmentations(&probes, &segmentations);
        let indices: Vec<usize> = plan.iter().map(|position| probes[*position].0).collect();
        assert_eq!(indices[0], 1);
        assert!(indices.contains(&0), "parser 首选保底：{plan:?}");
        assert!(indices.contains(&2), "结构代表保底：{plan:?}");
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
