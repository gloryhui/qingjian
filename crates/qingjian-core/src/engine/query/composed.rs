//! 未登录组合候选：词库里没有整词、但由现有字词能拼出来的组合。
//!
//! 词图能把 `ye'lang` 拼成 `[野][狼]`，但这条路径的分数天生低于 `夜郎` 这种完整词，
//! 按分数排序的整句 Top-K 永远轮不到它，用户第一次也就选不到。
//!
//! 这里不再按分数从路径池里捞，而是用**词库共现证据**当质量门槛重新在词格上找一遍：
//! 两个部分的交界字如果在**同一个词条**里出现过，这两个字语义上相关，它们拼出来的组合
//! 比同音字随机拼接更像一个词。`野`+`狼` 有 `狼子野心` 作证，`也`+`狼`、`野`+`浪`
//! 没有，于是有限的候选名额给的是前者。这不是给「野狼」写的规则：判据完全来自词库本身，
//! 换任何一对字都按同一条规则算。
//!
//! 除了词库共现，还有一条**个人证据**通道：用户选过这个组合之后，
//! [`crate::sentence::Conversion::personal_bonus`] 会大于零，此时不再要求共现证据，
//! 用户自己选的词优先于通用判据。
//!
//! 硬约束：完整覆盖全部音节、无占位、无模糊音 / 敲错代价、两个部分都是词库词、
//! 路径分落在最优路径的 [`COMPOSED_MARGIN`] 以内、与已有候选文本去重、
//! 一次最多 [`COMPOSED_CANDIDATES`] 条。

use std::sync::Arc;

use qingjian_dictionary::{Dictionary, SyllablePattern};

use super::super::{Engine, Learner};
use crate::candidate::{Candidate, CandidateKind};
use crate::fuzzy::Expanded;
use crate::parser::Segmentation;
use crate::ranking::weight_bonus;
use crate::sentence::{
    Context, Conversion, Personal, SentenceWord, SpanCache, SpanWord, fallback_log_prob,
    span_candidates, transition_log_prob,
};

/// 一次查询最多暴露几条未登录组合候选：够用户第一次就选中，又不会把候选窗口灌满。
pub(super) const COMPOSED_CANDIDATES: usize = 3;

/// 组合候选的路径分相对同一次查询最优整句分最多落后多少 nat。
///
/// 共现证据只回答「这两个字在词库里有没有关系」，回答不了「这次输入是不是就要这个组合」。
/// `nihao` 的 `呢好` 也能在 `好呢` 里共现，但 `你好` 是一个高频完整词，用户不会要一个拆读；
/// `ye'lang` 的 `夜郎` 只有 83 次，`野狼` 才是有意义的替代。两者的区别量在路径分上：
/// 整句路径分是 log 概率之和，一个完整词与「它拆成两个字」的差距 = 多摊的一次
/// [`FALLBACK_PENALTY`] 量级再加两段词频之比，在产品词库（总词频约 2.4e8）下，
/// 「已经有一个常用完整词」的输入差距都在 10.5 nat 以上，而「整词罕见、组合才是用户可能想要的」
/// 都在 9 nat 以内。取 10 就是这条分界，并且它随词典总词频自然缩放，不是为某个输入挑的数。
///
/// [`FALLBACK_PENALTY`]: crate::sentence::FALLBACK_PENALTY
pub(super) const COMPOSED_MARGIN: f64 = 10.0;

/// 共现索引只收这么多个字以内的词条：长词里隔得远的两个字同词出现多半是巧合。
const COMPOSITION_WORD_CHARS: usize = 6;

impl Engine {
    /// 从做过多路径搜索的切分里提取未登录组合候选，按个人选择次数与路径分排序，最多 [`COMPOSED_CANDIDATES`] 条。
    ///
    /// `best_score` 是这次联合搜索最优路径的分（有神经重排时已经是重排后的分）：
    /// 组合候选只在它 [`COMPOSED_MARGIN`] 以内才出。
    pub(super) fn composed_candidates(
        &self,
        search: &[(usize, Expanded)],
        segmentations: &[Segmentation],
        items: &[Candidate],
        typed: &str,
        best_score: f64,
    ) -> Vec<Conversion> {
        let dictionaries = self.all_dictionaries();
        let log_total = (self.total_frequency() as f64).max(1.0).ln();
        // 与词级排序同一个「这个输入串下选过什么」的键：双拼 / 注音下作用域是敲的键，
        // 选择记录记的是解码出来的拼音（见 `commit::consumed_by`），所以先解码再取字母。
        let keys = self.composition.scope();
        let decoded = self.decode(keys);
        let scope: &str = decoded.as_ref().map_or(keys, |d| d.pinyin());
        let letters = crate::engine::choice_key(scope, scope.len());
        let letters = &letters[..typed.len().min(letters.len())];
        let mut found: Vec<(u32, Conversion)> = Vec::new();
        for (index, expanded) in search {
            let segmentation = &segmentations[*index];
            // 只给打完了的切分造组合：末尾还差字母时整段边界本身就不确定，
            // 拿它拼出来的「词」只是半个音节的产物。
            if segmentation
                .syllables
                .iter()
                .any(|syllable| !syllable.complete)
            {
                continue;
            }
            self.composed_from_lattice(
                segmentation,
                expanded,
                &dictionaries,
                log_total,
                letters,
                &mut found,
            );
        }
        let mut kept: Vec<(u32, Conversion)> = found
            .into_iter()
            .filter(|(_, conversion)| {
                !items.iter().any(|item| item.text == conversion.text)
                    && conversion.score >= best_score - COMPOSED_MARGIN
            })
            .collect();
        // 同文本只留一条（不同切分可能拼出同一个词）
        kept.sort_by(|left, right| {
            right
                .1
                .score
                .total_cmp(&left.1.score)
                .then_with(|| left.1.text.cmp(&right.1.text))
        });
        kept.dedup_by(|left, right| left.1.text == right.1.text);
        // 用户在这个输入串下选过的排前面（与词级 `ranking::rank` 同一个键序），其余按路径分
        kept.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.score.total_cmp(&left.1.score))
                .then_with(|| left.1.text.cmp(&right.1.text))
        });
        kept.truncate(COMPOSED_CANDIDATES);
        let mut paths: Vec<Conversion> =
            kept.into_iter().map(|(_, conversion)| conversion).collect();
        self.rescore_ordered(&mut paths);
        // 神经分只重排「没有个人选择」的那部分：用户在这个输入串下明确选过的组合仍然排在前面，
        // 与词级 `ranking::rank` 的键序一致（选择次数优先于上下文分）。
        paths.sort_by(|left, right| {
            self.learner
                .choice_weight(letters, &right.text)
                .cmp(&self.learner.choice_weight(letters, &left.text))
                .then_with(|| right.score.total_cmp(&left.score))
                .then_with(|| left.text.cmp(&right.text))
        });
        paths
    }

    /// 一条切分的词格上所有「两个词 + 交界字共现」的组合。
    fn composed_from_lattice(
        &self,
        segmentation: &Segmentation,
        expanded: &Expanded,
        dictionaries: &[&Dictionary],
        log_total: f64,
        letters: &str,
        found: &mut Vec<(u32, Conversion)>,
    ) {
        let positions = expanded.positions();
        let n = crate::sentence::effective_len(&positions, false);
        if n < 2 || n != segmentation.syllables.len() {
            return;
        }
        let personal = self.personal();
        let cost = |position: usize, syllable: &str| expanded.cost(position, syllable);
        let weight = |text: &str| self.learner.weight(text);
        let mut cache = self.span_cache.borrow_mut();
        for split in 1..n {
            let heads = span_words(
                dictionaries,
                &positions[..split],
                0,
                personal,
                &weight,
                &cost,
                &mut cache,
            );
            let tails = span_words(
                dictionaries,
                &positions[split..n],
                split,
                personal,
                &weight,
                &cost,
                &mut cache,
            );
            for head in heads.iter() {
                for tail in tails.iter() {
                    if head.penalty > 0.0 || tail.penalty > 0.0 {
                        continue;
                    }
                    let Some(junction) = junction_chars(&head.text, &tail.text) else {
                        continue;
                    };
                    let supported = self.composes(junction.0, junction.1);
                    // 没有词库共现证据时，只有用户自己的学习数据能把它留下来；
                    // 一条个人数据都没有就没必要为它算分。
                    if !supported && !self.has_personal_evidence() {
                        continue;
                    }
                    let (conversion, choice) =
                        self.compose_pair(head, tail, split, log_total, letters, &weight);
                    if !supported && conversion.personal_bonus <= 0.0 {
                        continue;
                    }
                    found.push((choice, conversion));
                }
            }
        }
    }

    /// 把两个词拼成一条 [`Conversion`]，分数按整句路径同一套公式算（静态 LM + 个人 n-gram + 用户选择加分）。
    /// 返回的第二个值是用户在这个输入串下选过这个词几次，用来排序。
    fn compose_pair(
        &self,
        head: &SpanWord,
        tail: &SpanWord,
        tail_start: usize,
        log_total: f64,
        letters: &str,
        weight: &impl Fn(&str) -> u32,
    ) -> (Conversion, u32) {
        let model = &*self.language_model;
        let personal = self.personal();
        let head_fallback = fallback_log_prob(head.frequency, log_total);
        let step_head =
            transition_log_prob(model, personal, Context::START, &head.text, head_fallback);
        let tail_fallback = fallback_log_prob(tail.frequency, log_total);
        let step_tail = transition_log_prob(
            model,
            personal,
            Context::after(&head.text),
            &tail.text,
            tail_fallback,
        );
        let static_head = model.log_prob(None, &head.text).unwrap_or(head_fallback);
        let static_tail = model
            .log_prob(Some(&head.text), &tail.text)
            .unwrap_or(tail_fallback);
        let static_score = static_head + static_tail;
        // 与 Viterbi 一致：个人 n-gram 相对静态模型的增量 + 用户选择次数加分；代价为零（上面已挡掉）
        let personal_delta = (step_head - static_head) + (step_tail - static_tail);
        let selection_bonus = weight_bonus(weight(&head.text)) + weight_bonus(weight(&tail.text));
        let text = format!("{}{}", head.text, tail.text);
        let mut syllables = head.syllables.clone();
        syllables.extend(tail.syllables.iter().cloned());
        let choice = self.learner.choice_weight(letters, &text);
        debug_assert!(syllables.len() == tail_start + tail.syllables.len());
        (
            Conversion {
                text,
                syllables,
                words: vec![
                    SentenceWord {
                        text: head.text.clone(),
                        syllables: head.syllables.clone(),
                        placeholder: false,
                    },
                    SentenceWord {
                        text: tail.text.clone(),
                        syllables: tail.syllables.clone(),
                        placeholder: false,
                    },
                ],
                score: static_score + personal_delta + selection_bonus,
                static_score,
                personal_bonus: personal_delta + selection_bonus,
                penalty: 0.0,
            },
            choice,
        )
    }

    /// 两个字是否在某个词条里同词出现过。索引首次用到时从静态词库建一次，之后只读。
    pub(crate) fn composes(&self, left: char, right: char) -> bool {
        self.composition_pairs()
            .binary_search(&pair_key(left, right))
            .is_ok()
    }

    /// 有没有个人学习数据。没有就不必为「只靠个人证据才能成立」的组合算分。
    fn has_personal_evidence(&self) -> bool {
        self.personal().ngram.is_some()
    }

    /// 词库里「同词共现」的无序字对，升序排列。用户词不进来：它随学习变化，而这张表是只读索引；
    /// 用户自己造的词由词级查询直接命中，不靠组合候选兜底。
    fn composition_pairs(&self) -> &[u64] {
        self.composition_pairs.get_or_init(|| {
            let mut dictionaries: Vec<&Dictionary> = vec![&self.dictionary];
            dictionaries.extend(self.extra_dictionaries.iter());
            build_composition_pairs(&dictionaries).into()
        })
    }

    /// 拿到神经分的组合候选重排（异步还没到时保持原序）。
    fn rescore_ordered(&self, paths: &mut [Conversion]) {
        if paths.len() > 1 {
            self.rescore_paths(paths);
        }
    }
}

/// 组合两部分的交界字：前一个词的末字与后一个词的首字。
fn junction_chars(head: &str, tail: &str) -> Option<(char, char)> {
    Some((head.chars().next_back()?, tail.chars().next()?))
}

/// 无序字对的打包键：两个字各占 32 位，小的在前，这样 `野狼` 与 `狼野` 是同一个键。
fn pair_key(left: char, right: char) -> u64 {
    let (low, high) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    ((low as u64) << 32) | high as u64
}

/// 扫一遍静态词库，把「在同一个词条里出现过」的字对收成一个升序数组（二分查）。
///
/// 每个词条只在栈上摊开一次、不建 `Vec`：这个词库九万多条，一次查询里第一条需要组合候选的
/// 按键付一次这个代价（约 16 万对，几毫秒），之后是只读的。
fn build_composition_pairs(dictionaries: &[&Dictionary]) -> Vec<u64> {
    let mut buffer = ['\0'; COMPOSITION_WORD_CHARS + 1];
    let mut pairs: Vec<u64> = Vec::new();
    for dictionary in dictionaries {
        for entry in dictionary.entries() {
            let mut len = 0;
            for ch in entry.text.chars() {
                if len > COMPOSITION_WORD_CHARS {
                    break;
                }
                buffer[len] = ch;
                len += 1;
            }
            if !(2..=COMPOSITION_WORD_CHARS).contains(&len) {
                continue;
            }
            for (position, left) in buffer[..len].iter().enumerate() {
                for right in &buffer[position + 1..len] {
                    if left != right {
                        pairs.push(pair_key(*left, *right));
                    }
                }
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// 词图一个格子的候选，与整句转换走同一个 [`SpanCache`]：同一次查询里 Viterbi 查过的格子直接命中。
#[allow(clippy::too_many_arguments)]
fn span_words(
    dictionaries: &[&Dictionary],
    span: &[Vec<SyllablePattern<'_>>],
    start: usize,
    personal: Personal<'_>,
    weight: &impl Fn(&str) -> u32,
    cost: &impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
) -> Arc<[SpanWord]> {
    cache.get_or_insert_with(SpanCache::key(span), || {
        span_candidates(dictionaries, span, start, personal, weight, cost)
    })
}

/// 组合候选插到候选表里的位置：开头英文候选之后、覆盖整段输入的完整词之后、整句首选之后。
/// 组合是词库没收录时的补位，不能挤到已有的完整词或整句首选的上面去。
pub(super) fn composed_insert_position(items: &[Candidate], typed: &str) -> usize {
    let mut position = super::leading_english(items);
    if items.get(position).is_some_and(|candidate| {
        matches!(
            candidate.kind,
            CandidateKind::Sentence | CandidateKind::Cloud | CandidateKind::Code
        )
    }) {
        position += 1;
    }
    position
        + items[position.min(items.len())..]
            .iter()
            .take_while(|candidate| {
                candidate.kind == CandidateKind::Chinese && candidate.syllables.concat() == typed
            })
            .count()
}
