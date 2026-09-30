//! 未登录组合候选：词库里没有整词、但由现有字词能拼出来的组合。
//!
//! 词图能把 `ye'lang` 拼成 `[野][狼]`，但这条路径的分数天生低于 `夜郎` 这种完整词，
//! 按分数排序的整句 Top-K 永远轮不到它，用户第一次也就选不到。
//!
//! 所以这里不按分数从整句路径池里捞，而是**在词格上按结构条件重新找一遍**：
//! 完整覆盖全部音节、无占位、两个部分都是词库里的词、没有模糊音 / 敲错代价、
//! 路径分落在最优路径的 [`COMPOSED_MARGIN`] 以内、与已有候选去重、一次最多
//! [`COMPOSED_CANDIDATES`] 条。
//!
//! **准入不看「这两个字有没有在别的词条里一起出现过」**。那个信号（`野`+`狼` ← `狼子野心`）
//! 回答不了「这次输入是不是就要这个组合」：`藤壶`、`云豹`、`纸杯`、`纸伞`、`石阶` 都是正常汉语组合，
//! 词库暂时没收，不能因为两个字没在别的词条里同框就不给候选。词库继续漏词是词库的事（见 #14），
//! 解码器这一层必须保证「合理组合第一次也能选出来」。
//!
//! 共现证据降级为**排序加成**：同样合格的两条组合，交界字在词库里同词出现过的那条排前面
//! （`ye'lang` 下 `野狼` 因此排在 `也浪` 前）。它只影响顺序，不影响谁有资格存在。
//!
//! 另外两条排序依据：用户在这个输入串下明确选过的排最前（personal / choice evidence），
//! 之后按整句路径分。用户选过之后组合还会进个人 n-gram，见 [`crate::sentence::Personal`]。

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
/// 这是准入唯一与分数有关的条件，作用是把「这次输入根本解释不出这个组合」的路径挡在外面：
/// 整句路径分是 log 概率之和，一个完整词与「它拆成两个字」的差距 = 多摊的一次
/// [`FALLBACK_PENALTY`] 量级再加两段词频之比。在产品词库（总词频约 2.4e8）下，
/// 已经有一个常用完整词的输入差距都在 10.5 nat 以上（`nihao`/`呢好` -11.5、`dangao`/`蛋高` -10.9），
/// 而整词罕见、组合才是用户可能想要的都在 9 nat 以内（`ye'lang`/`野狼` -8.6、`jing'shui`/`井水` -7.7）。
/// 取 10 就是这条分界，它随词典总词频自然缩放。名额上限 [`COMPOSED_CANDIDATES`] 负责限量。
///
/// [`FALLBACK_PENALTY`]: crate::sentence::FALLBACK_PENALTY
pub(super) const COMPOSED_MARGIN: f64 = 10.0;

/// 共现索引只收这么多个字以内的词条：长词里隔得远的两个字同词出现多半是巧合。
const COMPOSITION_WORD_CHARS: usize = 6;

/// 组合候选看每个跨度时最多取几个词。整句词图只留 [`crate::sentence::SPAN_CANDIDATES`] 个
/// （按词频），像 `藤`、`壶`、`豹`、`石`、`阶` 这种词频排在后面的字根本进不来；组合候选是
/// 「词库没收录时的补位」，本来就该看得比词图宽一点。仍然有界，且走独立的缓存键，
/// 不改变整句词图的格子内容。
const COMPOSED_PART_CANDIDATES: usize = 12;

/// 整段拼音已经有一个**常用完整词**读法时，不再出组合候选。
///
/// 组合候选是「词库没收录时的补位」：最优路径整段就是一个词库里的词、而且它足够常用时，
/// 用户要的多半就是它，再摆一排拆读只会把正常候选挤下去（`nihao` 的 `你号`、`zhongguo` 的 `中过`）。
/// 反过来，整词生僻（`ye'lang` 的 `夜郎` 83 次）或整段根本没有完整词（`teng'hu`）时，
/// 拆读才是用户可能想要的。
///
/// 门槛按词频定，产品词库上的分布是断开的：常用词（`蛋糕` 13914、`没关系` 14807、
/// `你好` 71960、`世界` 114813）与生僻词（`治三` 16、`夜郎` 83、`主桥` 164、`坐镇` 208、
/// `制备` 903）差一个数量级以上，门槛取在中间任何位置结果都一样。
const COMPOSED_DOMINANT_WORD_FREQUENCY: u32 = 3000;

/// 只给这么多个音节以内的切分造组合：组合候选是**词**层面的补位（新词、人名、店名、术语），
/// 整句长度上的「两个词拼起来」属于整句路径该管的事。
const COMPOSED_MAX_SYLLABLES: usize = 6;

impl Engine {
    /// 从做过多路径搜索的切分里提取未登录组合候选，按个人选择次数与路径分排序，最多 [`COMPOSED_CANDIDATES`] 条。
    ///
    /// `best` 是这次联合搜索最优路径（**重排前**）：组合候选只在它 [`COMPOSED_MARGIN`] 以内才出，
    /// 而组合自己的分也是重排前的，两边同一把尺子；它整段是一个常用词时直接不出组合候选。
    pub(super) fn composed_candidates(
        &self,
        search: &[(usize, Expanded)],
        segmentations: &[Segmentation],
        items: &[Candidate],
        typed: &str,
        best: &Conversion,
    ) -> Vec<Conversion> {
        let dictionaries = self.all_dictionaries();
        let log_total = (self.total_frequency() as f64).max(1.0).ln();
        let best_score = best.score;
        if dominant_complete_word(best, log_total) {
            return Vec::new();
        }
        // 与词级排序同一个「这个输入串下选过什么」的键：双拼 / 注音下作用域是敲的键，
        // 选择记录记的是解码出来的拼音（见 `commit::consumed_by`），所以先解码再取字母。
        let keys = self.composition.scope();
        let decoded = self.decode(keys);
        let scope: &str = decoded.as_ref().map_or(keys, |d| d.pinyin());
        let letters = crate::engine::choice_key(scope, scope.len());
        let letters = &letters[..typed.len().min(letters.len())];
        let mut found: Vec<Composed> = Vec::new();
        for (index, _) in search {
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
                &dictionaries,
                log_total,
                letters,
                best_score,
                &mut found,
            );
        }
        let mut kept: Vec<Composed> = found
            .into_iter()
            .filter(|composed| {
                !items
                    .iter()
                    .any(|item| item.text == composed.conversion.text)
                    && composed.conversion.score >= best_score - COMPOSED_MARGIN
            })
            .collect();
        // 同文本只留一条（不同切分可能拼出同一个词），留下分最高的那条
        kept.sort_by(|left, right| {
            right
                .conversion
                .score
                .total_cmp(&left.conversion.score)
                .then_with(|| left.conversion.text.cmp(&right.conversion.text))
        });
        kept.dedup_by(|left, right| left.conversion.text == right.conversion.text);
        sort_composed(&mut kept);
        #[cfg(test)]
        {
            *self.last_composed_pool.borrow_mut() = kept
                .iter()
                .map(|composed| composed.conversion.text.clone())
                .collect();
        }
        kept.truncate(COMPOSED_CANDIDATES);
        let mut paths: Vec<Conversion> = kept
            .iter()
            .map(|composed| composed.conversion.clone())
            .collect();
        // 只更新分数、不在这里排序：排序交给 `sort_composed`，那边还要看个人选择与共现加成，
        // 而且 `rescore_paths` 会把 `paths` 就地重排，打乱与 `kept` 的一一对应。
        self.rescore_path_scores(&mut paths);
        for (composed, rescored) in kept.iter_mut().zip(paths) {
            composed.conversion.score = rescored.score;
        }
        sort_composed(&mut kept);
        kept.into_iter()
            .map(|composed| composed.conversion)
            .collect()
    }

    /// 一条切分的词格上所有「两个词拼起来」的组合。
    ///
    /// 词格只按**敲的原样读音**建：模糊音与敲错变体是「用户可能敲错了」的猜测，拿它拼出来的
    /// 新词没有依据（Issue #13 也要求不许把高代价的 typo 路径变成新词）。只按原样还有个直接好处：
    /// 格子的名额不会被 `与`、`于` 这种靠敲错边命中的高频字占满。
    fn composed_from_lattice(
        &self,
        segmentation: &Segmentation,
        dictionaries: &[&Dictionary],
        log_total: f64,
        letters: &str,
        best_score: f64,
        found: &mut Vec<Composed>,
    ) {
        let positions: Vec<Vec<SyllablePattern<'_>>> = segmentation
            .patterns()
            .into_iter()
            .map(|pattern| vec![pattern])
            .collect();
        let n = crate::sentence::effective_len(&positions, false);
        if n < 2 || n != segmentation.syllables.len() || n > COMPOSED_MAX_SYLLABLES {
            return;
        }
        let personal = self.personal();
        let cost = |_position: usize, _syllable: &str| 0.0;
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
            // 两段式：先只算分（不建字符串、不查选择次数），排序后只给前几条建 [`Conversion`]。
            // 一个 2 音节输入有上百个配对，全部建对象是这一段的全部开销所在。
            let mut scored: Vec<PairScore<'_>> = Vec::new();
            for head in heads.iter() {
                let head_fallback = fallback_log_prob(head.frequency, log_total);
                for tail in tails.iter() {
                    let Some((left, right)) = junction_chars(&head.text, &tail.text) else {
                        continue;
                    };
                    let tail_fallback = fallback_log_prob(tail.frequency, log_total);
                    // 一元下界先挡掉明显够不到 margin 的配对
                    if head_fallback + tail_fallback < best_score - COMPOSED_MARGIN {
                        continue;
                    }
                    let Some(score) =
                        self.pair_score(head, tail, head_fallback, tail_fallback, &weight)
                    else {
                        continue;
                    };
                    if score < best_score - COMPOSED_MARGIN {
                        continue;
                    }
                    scored.push((head, tail, score, self.composes(left, right)));
                }
            }
            scored.sort_by(|left, right| {
                composed_rank(right.2, right.3)
                    .total_cmp(&composed_rank(left.2, left.3))
                    .then_with(|| left.0.text.cmp(&right.0.text))
                    .then_with(|| left.1.text.cmp(&right.1.text))
            });
            scored.truncate(COMPOSED_SPLIT_KEEP);
            for (head, tail, score, supported) in scored {
                let conversion = self.compose_pair(head, tail, split, score, letters);
                found.push(Composed {
                    choice: self.learner.choice_weight(letters, &conversion.text),
                    supported,
                    conversion,
                });
            }
        }
    }

    /// 配对分数，按整句路径同一套公式算（静态 LM + 个人 n-gram + 用户选择加分），**不建字符串**。
    fn pair_score(
        &self,
        head: &SpanWord,
        tail: &SpanWord,
        head_fallback: f64,
        tail_fallback: f64,
        weight: &impl Fn(&str) -> u32,
    ) -> Option<f64> {
        let model = &*self.language_model;
        let personal = self.personal();
        let step_head =
            transition_log_prob(model, personal, Context::START, &head.text, head_fallback);
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
        // 与 Viterbi 一致：个人 n-gram 相对静态模型的增量 + 用户选择次数加分；代价为零（只按原样读音建格）
        let personal_delta = (step_head - static_head) + (step_tail - static_tail);
        let selection = weight_bonus(weight(&head.text)) + weight_bonus(weight(&tail.text));
        Some(static_head + static_tail + personal_delta + selection)
    }

    /// 配对的静态分：语言模型认识就用模型的，不认识用词频兜底。
    fn pair_static_score(&self, head: &SpanWord, tail: &SpanWord) -> f64 {
        let model = &*self.language_model;
        let log_total = (self.total_frequency() as f64).max(1.0).ln();
        let head_fallback = fallback_log_prob(head.frequency, log_total);
        let tail_fallback = fallback_log_prob(tail.frequency, log_total);
        let static_head = model.log_prob(None, &head.text).unwrap_or(head_fallback);
        let static_tail = model
            .log_prob(Some(&head.text), &tail.text)
            .unwrap_or(tail_fallback);
        static_head + static_tail
    }

    /// 把两个词拼成一条 [`Conversion`]，分数由 [`Self::pair_score`] 算好传进来。
    fn compose_pair(
        &self,
        head: &SpanWord,
        tail: &SpanWord,
        tail_start: usize,
        score: f64,
        _letters: &str,
    ) -> Conversion {
        let static_score = self.pair_static_score(head, tail);
        let text = format!("{}{}", head.text, tail.text);
        let mut syllables = head.syllables.clone();
        syllables.extend(tail.syllables.iter().cloned());
        debug_assert!(syllables.len() == tail_start + tail.syllables.len());
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
            score,
            static_score,
            personal_bonus: score - static_score,
            penalty: 0.0,
        }
    }

    /// 两个字是否在某个词条里同词出现过。索引首次用到时从静态词库建一次，之后只读。
    pub(crate) fn composes(&self, left: char, right: char) -> bool {
        self.composition_pairs()
            .binary_search(&pair_key(left, right))
            .is_ok()
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
}

/// 最优路径整段就是一个词库里的词、而且它不低于 [`COMPOSED_DOMINANT_WORD_FREQUENCY`]：
/// 这次输入已经有一个常用读法，组合候选不必再出。
fn dominant_complete_word(best: &Conversion, log_total: f64) -> bool {
    best.word_count() == 1
        && !best.altered()
        && !best.has_placeholder()
        && best.score >= fallback_log_prob(COMPOSED_DOMINANT_WORD_FREQUENCY, log_total)
}

/// 一条待选的组合候选：用户在这个输入串下选过几次、交界字有没有词库共现证据、以及路径本身。
struct Composed {
    choice: u32,
    supported: bool,
    conversion: Conversion,
}

/// 排序阶段的一个配对：只带分数与引用，不建字符串。
type PairScore<'a> = (&'a SpanWord, &'a SpanWord, f64, bool);

/// 每个切分点最多给几个配对建 [`Conversion`]：配对分数已经算完，这一步只是把
/// 还活着的少量候选变成对象。取 8 足够覆盖最终最多 3 条曝光加上并列。
const COMPOSED_SPLIT_KEEP: usize = 24;

/// 交界字在词库里同词出现过时给的排序加成（nat）。
///
/// 它只是加成、不是资格：一个完整词与「它拆成两个字」在整句路径分上的差距 = 多摊的一次
/// [`FALLBACK_PENALTY`] 量级再加两段词频之比，`野狼` 比 `也浪` 低 5.2 nat，
/// 取 6 正好让有词库共现证据的那条浮上来，同时**盖不过真正强的证据**——
/// 语言模型或神经模型给出 10 nat 量级的接续分时照样能把它压回去。
///
/// [`FALLBACK_PENALTY`]: crate::sentence::FALLBACK_PENALTY
const COMPOSED_SUPPORT_BONUS: f64 = 6.0;

/// 组合候选的排序键，与词级 `ranking::rank` 同序：
/// 用户在这个输入串下明确选过的 > 路径分（含共现加成）。神经分更新的是路径分那一档。
fn sort_composed(composed: &mut [Composed]) {
    composed.sort_by(|left, right| {
        right
            .choice
            .cmp(&left.choice)
            .then_with(|| rank(right).total_cmp(&rank(left)))
            .then_with(|| left.conversion.text.cmp(&right.conversion.text))
    });
}

/// 排序用的分数：路径分 + 共现加成。准入用的 [`Composed::conversion`] 分不带这个加成。
fn rank(composed: &Composed) -> f64 {
    composed_rank(composed.conversion.score, composed.supported)
}

/// 排序用的分数：路径分 + 共现加成。
fn composed_rank(score: f64, supported: bool) -> f64 {
    score
        + if supported {
            COMPOSED_SUPPORT_BONUS
        } else {
            0.0
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
    // 与整句词图用同一张缓存但不同的键：格子内容不同（看得更宽），不能互相覆盖
    let key = format!("{COMPOSED_PART_CANDIDATES}\x01{}", SpanCache::key(span));
    cache.get_or_insert_with(key, || {
        span_candidates(
            dictionaries,
            span,
            start,
            personal,
            weight,
            cost,
            COMPOSED_PART_CANDIDATES,
        )
    })
}

/// 组合候选插到候选表里的位置：开头英文候选之后、整句首选之后、覆盖整段输入的完整词之后，
/// **并且永远不占第一条**。
///
/// 组合是「词库没收录时的补位」：词库自己给出的候选（含模糊音、前缀命中）永远先看一遍，
/// 猜测性的拆读不抢已经排好的第一位。`kaiha` 开了 `f_h` 时 `开放` 仍排第一，`野狼` 排在 `夜郎` 之后。
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
    let covered = items[position.min(items.len())..]
        .iter()
        .take_while(|candidate| {
            candidate.kind == CandidateKind::Chinese && candidate.syllables.concat() == typed
        })
        .count();
    (position + covered).max(usize::from(!items.is_empty()))
}
