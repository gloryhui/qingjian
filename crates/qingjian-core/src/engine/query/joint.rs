//! 多种拼音切分共享整句评分和神经重排，首选切分随首选候选返回。
//!
//! # 为什么不是「固定前 N 条切分」
//!
//! 旧实现只让 parser 排序靠前的固定两条切分进入整句路径搜索，其余切分哪怕词格和语言模型
//! 证据明显更强，也没有机会参与竞争——「正确的解释排在第三条」就是死路。
//!
//! 现在分三步，预算跟着证据走：
//!
//! 1. **廉价证据**（[`cheap_evidence`]）：**每条有资格的切分**都算一份，只用已经排好的词级候选
//!    `items`（它们是所有切分的词库命中汇总）沿这条切分贪心找最长的可选词往前推，
//!    零额外词库查询、不看 parser 名次。一个音节串能用更少的词盖住，说明词库更认这条切分。
//! 2. **选整段探针名额**（[`select_probe_candidates`]）：整段窄搜索（单前驱、不带路线串）是
//!    真正花钱的一步——每条 9 音节切分约 480 µs 的词格——所以名额有上限
//!    [`JOINT_PROBE_LIMIT`]。名额按**廉价证据**分配：先给每个采样签名一个代表，再按证据全局
//!    填满。采样签名只是「音节数 + 不完整音节数」，**不是语义等价类**：同签名的切分照样各自
//!    带证据参与排序，证据更强的那个才是代表。
//! 3. **按探针分分配展开名额**：探针分最高的切分一定展开，parser 首选与**音节边界**不同的最佳探针
//!    作为结构代表保底，其余按探针分从高到低填，直到落后最优探针超过 [`JOINT_PROBE_SLACK`]
//!    或碰到硬上限 [`JOINT_MAX_SEGMENTATIONS`]。
//!
//! 所以决定生死的是「这条切分自己的词法证据 + 整段词格能给出多好的路径」，不是它在 parser
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

/// 一次查询最多给几条切分做**整段**探针。
///
/// 整段探针唯一真正花掉的是**词格**：每条切分的前后缀不同，格子只能部分共享，
/// 实测每多探一条 9 音节切分，整串查询多约 480 µs。所以名额必须有上限。
///
/// 名额给谁由[`cheap_evidence`]决定，不是 parser 名次：第 5 条以后只要词法证据更强，
/// 照样挤得掉前面的人（见 [`select_probe_candidates`]）。
const JOINT_PROBE_LIMIT: usize = 4;

/// 一条切分的探针结果：`depth` 个音节上的最优路径。
struct Probe {
    index: usize,
    depth: usize,
    best: Conversion,
}

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
        let probes =
            self.probe_segmentations(segmentations, &eligible, items, &dictionaries, typos);
        let plan = plan_joint_segmentations(&probes, segmentations);
        let mut ranked = Vec::new();
        let mut search: Vec<(usize, Expanded)> = Vec::with_capacity(plan.len());
        for position in plan {
            let probe = &probes[position];
            let index = probe.index;
            let segmentation = &segmentations[index];
            let expanded = self.expand_positions(&segmentation.patterns(), typos);
            if k == 1 {
                // 探针在整段上跑的就是同一条窄搜索，结果直接复用，不再跑第二遍；
                // 只探了前缀（长输入）时前缀路径不是整句，扔掉重跑。
                if probe.depth >= segmentation.syllables.len() && covers(&probe.best, segmentation)
                {
                    ranked.push((index, probe.best.clone()));
                } else if let Some(path) = self.full_path(segmentation, &expanded, &dictionaries) {
                    ranked.push((index, path));
                }
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
        ranked.sort_by(|a, b| {
            b.1.score
                .total_cmp(&a.1.score)
                .then_with(|| a.1.text.cmp(&b.1.text))
                .then_with(|| a.0.cmp(&b.0))
        });
        // 组合候选的门槛要看**重排前**的最优路径：先留一份，`k == 1` 时它会被 `remove` 掉
        let best = ranked[0].1.clone();
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
        let generated = self.composed_candidates(&search, segmentations, items, &typed, &best);
        self.joint_stats.set(JointStats {
            generated_candidates: generated.len(),
            ..self.joint_stats.get()
        });
        Some(JointOutcome { winner, generated })
    }

    /// 两阶段预算：先给**每条**有资格的切分算一份廉价证据，再把有限的整段探针名额
    /// 按这份证据分配（见 [`select_probe_candidates`]），最后只对拿到名额的切分跑整段窄搜索。
    ///
    /// 探针用的词格进同一个缓存，整句转换随后直接命中，不重复查词库。
    fn probe_segmentations(
        &self,
        segmentations: &[Segmentation],
        eligible: &[usize],
        items: &[Candidate],
        dictionaries: &[&Dictionary],
        typos: bool,
    ) -> Vec<Probe> {
        let weight = |text: &str| self.learner.weight(text);
        let evidence: Vec<CheapEvidence> = eligible
            .iter()
            .map(|index| cheap_evidence(items, &segmentations[*index], &weight))
            .collect();
        let chosen = select_probe_candidates(eligible, segmentations, &evidence, JOINT_PROBE_LIMIT);
        let mut probes = Vec::with_capacity(chosen.len());
        let mut probed = Vec::with_capacity(chosen.len());
        for slot in chosen {
            let index = eligible[slot];
            let depth = segmentations[index].syllables.len();
            probed.push(index);
            if let Some(probe) = self.probe_prefix(segmentations, index, depth, dictionaries, typos)
            {
                probes.push(probe);
            }
        }
        #[cfg(test)]
        {
            *self.last_probed_segmentations.borrow_mut() = probed;
            *self.last_cheap_evidence.borrow_mut() = eligible
                .iter()
                .zip(&evidence)
                .map(|(index, evidence)| (*index, evidence.average_word_length, evidence.strength))
                .collect();
        }
        probes
    }

    /// 一条切分前 `depth` 个音节上的最优路径。跨度缓存与整句转换共用，加深时只多算新的一层。
    fn probe_prefix(
        &self,
        segmentations: &[Segmentation],
        index: usize,
        depth: usize,
        dictionaries: &[&Dictionary],
        typos: bool,
    ) -> Option<Probe> {
        let segmentation = &segmentations[index];
        let expanded = self.expand_positions(&segmentation.patterns(), typos);
        let positions = expanded.positions();
        let depth = depth.min(sentence::effective_len(&positions, false));
        if depth == 0 {
            return None;
        }
        let groups = sentence::convert_path_groups(
            dictionaries,
            &positions[..depth],
            false,
            1,
            &*self.language_model,
            self.personal(),
            |text| self.learner.weight(text),
            |position, syllable| expanded.cost(position, syllable),
            &mut self.span_cache.borrow_mut(),
        );
        // 组内第一条就是这条路线的最高分路径（节点已按分数降序）
        let best = groups
            .into_iter()
            .next()
            .and_then(|group| group.into_iter().next())?;
        Some(Probe { index, depth, best })
    }

    /// 在整段音节上重跑一次窄搜索，拿覆盖全部音节的路径（前缀探针不够用时用）。
    fn full_path(
        &self,
        segmentation: &Segmentation,
        expanded: &Expanded,
        dictionaries: &[&Dictionary],
    ) -> Option<Conversion> {
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
        groups
            .into_iter()
            .next()
            .and_then(|group| group.into_iter().next())
            .filter(|path| covers(path, segmentation))
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

/// 一条切分的**廉价词法证据**：只用已经排好的词级候选 `items`，沿这条切分从第一个音节起
/// 贪心找最长的可选词往前推。
///
/// 零额外词库查询，也完全不看 parser 名次：一个音节串能用**更少的词**盖住，说明词库更认这条切分
/// （`an xi an an` 用 `安溪` + `安安` 两个词盖住，`an xian an` 只能一个字一个字盖）。
/// 同覆盖率时看强度：用到的词在词级候选里排得越前、用户选过越多，证据越强。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct CheapEvidence {
    /// 盖住整段时的**平均词长**（音节数）：词库里越认这条切分，能用的词越长、用到的词越少。
    /// 一个词都没用上就是 0。
    average_word_length: f64,

    /// 用到的词在词级候选里的强度之和：名次越靠前越大，再加用户选择次数。
    strength: f64,
}

impl CheapEvidence {
    /// 平均词长越长越强；一样长时看强度（`f64` 没有 `Ord`，单独写一个比较）。
    fn is_stronger_than(&self, other: &Self) -> bool {
        match self
            .average_word_length
            .total_cmp(&other.average_word_length)
        {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => self.strength > other.strength,
        }
    }
}

/// 沿一条切分在词级候选里贪心最长覆盖。`items` 是所有切分的命中汇总，
/// 用「音节序列与这条切分从当前位置起完全一致」来认它属于这条切分。
fn cheap_evidence(
    items: &[Candidate],
    segmentation: &Segmentation,
    weight: &impl Fn(&str) -> u32,
) -> CheapEvidence {
    let readings: Vec<&str> = segmentation
        .syllables
        .iter()
        .map(|syllable| syllable.text.as_str())
        .collect();
    let total = readings.len();
    if total == 0 {
        return CheapEvidence::default();
    }
    let mut position = 0;
    let mut covered = 0;
    let mut words = 0;
    let mut strength = 0.0;
    while position < total {
        let mut best: Option<(usize, f64)> = None;
        for (rank, item) in items.iter().enumerate() {
            if item.kind != CandidateKind::Chinese {
                continue;
            }
            let length = item.syllables.len();
            if length == 0 || position + length > total {
                continue;
            }
            if item
                .syllables
                .iter()
                .zip(&readings[position..position + length])
                .any(|(candidate, reading)| candidate != reading)
            {
                continue;
            }
            // 名次越靠前越强（词级排序已经算进词频、选择次数与上下文分）
            let score = 1.0 / (rank as f64 + 1.0) + f64::from(weight(&item.text));
            let better = match best {
                Some((best_length, best_score)) => {
                    length > best_length || (length == best_length && score > best_score)
                }
                None => true,
            };
            if better {
                best = Some((length, score));
            }
        }
        match best {
            Some((length, score)) => {
                position += length;
                covered += length;
                words += 1;
                strength += score;
            }
            // 这个词库在这个位置一个字都没有（罕见的残缺音节）：跳过，不计覆盖率
            None => position += 1,
        }
    }
    CheapEvidence {
        average_word_length: if words == 0 {
            0.0
        } else {
            covered as f64 / words as f64
        },
        strength,
    }
}

/// 选哪几条切分做整段探针，返回 `eligible` 的下标。
///
/// [`SamplingKey`] **只用来采样**（保证不同形状的切分都有机会），不用它判定语义等价：
/// 同一组里的成员各自带着自己的廉价证据参与排序，证据更强的那个才是这一组的代表。
/// 第一轮每个结构签名取一个代表（按证据降序，所以组数超过名额时是证据强的组先进），
/// 第二轮按证据全局填满剩余名额，同组的其他成员这时也能进。
fn select_probe_candidates(
    eligible: &[usize],
    segmentations: &[Segmentation],
    evidence: &[CheapEvidence],
    limit: usize,
) -> Vec<usize> {
    if eligible.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..eligible.len()).collect();
    order.sort_by(|left, right| {
        let stronger = evidence[*right].is_stronger_than(&evidence[*left]);
        let weaker = evidence[*left].is_stronger_than(&evidence[*right]);
        if stronger {
            std::cmp::Ordering::Greater
        } else if weaker {
            std::cmp::Ordering::Less
        } else {
            eligible[*left].cmp(&eligible[*right])
        }
    });
    let mut chosen: Vec<usize> = Vec::with_capacity(limit);
    let mut seen: Vec<SamplingKey> = Vec::new();
    for &slot in &order {
        if chosen.len() >= limit {
            return chosen;
        }
        let key = sampling_key(&segmentations[eligible[slot]]);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        chosen.push(slot);
    }
    for &slot in &order {
        if chosen.len() >= limit {
            break;
        }
        if !chosen.contains(&slot) {
            chosen.push(slot);
        }
    }
    chosen
}

/// [`select_probe_candidates`] 的测试入口。
#[cfg(test)]
pub(crate) fn select_probe_candidates_for_test(
    eligible: &[usize],
    segmentations: &[Segmentation],
    evidence: &[(f64, f64)],
    limit: usize,
) -> Vec<usize> {
    let evidence: Vec<CheapEvidence> = evidence
        .iter()
        .map(|(average_word_length, strength)| CheapEvidence {
            average_word_length: *average_word_length,
            strength: *strength,
        })
        .collect();
    select_probe_candidates(eligible, segmentations, &evidence, limit)
}

/// 采样用的结构签名：音节数、不完整音节数。
///
/// **只用来采样**（[`select_probe_candidates`] 的第一轮保证每种形状都有机会），
/// 不是语义等价类：同一个签名下音节边界并不相同（`xi lia …` 与 `xi li a …` 都是 9 个完整音节），
/// 谁更值得探由各自的[`cheap_evidence`]说了算。
type SamplingKey = (usize, usize);

fn sampling_key(segmentation: &Segmentation) -> SamplingKey {
    (
        segmentation.syllables.len(),
        segmentation.incomplete_count(),
    )
}

/// 音节边界签名：各音节的字母数。**这才是「结构不同」的定义**——`dang ao` 与 `dan gao`
/// 都是 2 个完整音节，但边界不一样，词格也就不同，做结构代表时不能当成同一个。
type BoundaryKey = Vec<usize>;

fn boundary_key(segmentation: &Segmentation) -> BoundaryKey {
    segmentation
        .syllables
        .iter()
        .map(|syllable| syllable.text.len())
        .collect()
}

/// 按探针分挑出真正要做多路径搜索的切分，返回 `probes` 里的下标（已按探针分降序）。
///
/// 保底三个代表先入：探针分最高的切分（证据最强）、parser 的首选（与旧行为连续）、
/// 以及结构签名不同于最优探针的最佳切分（切分结构代表）。其余按探针分从高到低填，
/// 落后最优探针超过 [`JOINT_PROBE_SLACK`] 就停。
fn plan_joint_segmentations(probes: &[Probe], segmentations: &[Segmentation]) -> Vec<usize> {
    if probes.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..probes.len()).collect();
    order.sort_by(|left, right| {
        probes[*right]
            .best
            .score
            .total_cmp(&probes[*left].best.score)
            .then_with(|| probes[*left].index.cmp(&probes[*right].index))
    });
    let mut chosen: Vec<usize> = Vec::with_capacity(JOINT_MAX_SEGMENTATIONS);
    let take = |position: usize, chosen: &mut Vec<usize>| {
        if chosen.len() < JOINT_MAX_SEGMENTATIONS && !chosen.contains(&position) {
            chosen.push(position);
        }
    };
    take(order[0], &mut chosen);
    if let Some(position) = probes.iter().position(|probe| probe.index == 0) {
        take(position, &mut chosen);
    }
    // 结构代表按**音节边界**选，不按采样用的粗签名：`dang ao` 与 `dan gao` 音节数相同、
    // 边界不同，是两条真正不同的词格，不能互相代表。
    let best_boundary = boundary_key(&segmentations[probes[order[0]].index]);
    if let Some(&position) = order
        .iter()
        .find(|position| boundary_key(&segmentations[probes[**position].index]) != best_boundary)
    {
        take(position, &mut chosen);
    }
    for &position in &order {
        if chosen.len() >= JOINT_SEGMENTATIONS_FLOOR {
            break;
        }
        take(position, &mut chosen);
    }
    let floor = probes[order[0]].best.score - JOINT_PROBE_SLACK;
    for &position in &order {
        if chosen.len() >= JOINT_MAX_SEGMENTATIONS || probes[position].best.score < floor {
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

    fn probe_scores(scores: &[(usize, f64)]) -> Vec<Probe> {
        scores
            .iter()
            .map(|(index, score)| Probe {
                index: *index,
                depth: 3,
                best: route("", &[""], *score, *score, 0.0),
            })
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
        assert_eq!(probes[plan[0]].index, 2, "探针分最高的切分先展开：{plan:?}");
        assert!(
            plan.iter().any(|position| probes[*position].index == 0),
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
        let indices: Vec<usize> = plan
            .iter()
            .map(|position| probes[*position].index)
            .collect();
        assert!(!indices.contains(&2), "落后太多不展开：{plan:?}");
        assert!(!indices.contains(&3), "落后太多不展开：{plan:?}");
    }

    #[test]
    fn plan_keeps_a_different_structure_as_representative() {
        let segmentations = vec![
            segmentation(&["ken", "eng"]),
            segmentation(&["ke", "neng"]),
            segmentation(&["ke", "n", "eng"]),
        ];
        let probes = probe_scores(&[(0, -9.0), (1, -8.0), (2, -9.2)]);
        let plan = plan_joint_segmentations(&probes, &segmentations);
        let indices: Vec<usize> = plan
            .iter()
            .map(|position| probes[*position].index)
            .collect();
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
