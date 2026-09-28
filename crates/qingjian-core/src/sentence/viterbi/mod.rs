//! 词图上的最优路径：bigram Viterbi + 束搜索。
//!
//! 状态只按前一个词分（束宽内），个人三元要的前二词取前驱节点的回指（它那条最优路径上的前一个词）：
//! 不扩状态，代价是三元上下文是近似的，个人数据量下够用。

use qingjian_dictionary::{Dictionary, Match, SyllablePattern};

use super::diversity::representative_indices;
use super::{
    ABBREVIATED_SPAN_CANDIDATES, BEAM_WIDTH, Context, Conversion, LanguageModel,
    MAX_WORD_SYLLABLES, MIN_PARTIAL_LETTERS, PathDiagnostic, Personal, SPAN_CANDIDATES,
    SearchDiagnostics, SentenceWord, SpanCache, SpanDiagnostic, SpanWord, TransitionDiagnostic,
    fallback_log_prob, transition_log_prob,
};
use crate::ranking::weight_bonus;

/// 词库里没有的孤立音节（罕见音节没有单字）按这个 log 概率兜底，让路径总能走通。
const UNKNOWN_LOG_PROB: f64 = -30.0;

/// 一条部分路径的末尾节点。
struct Node {
    /// 这个词从第几个音节开始。
    start: usize,

    /// 词。
    text: String,

    /// 词路径，以 NUL 分隔；共同前缀后的分歧在后续词格仍可辨认。
    route: String,

    /// 词的音节。
    syllables: Vec<String>,

    /// 到此为止的累计得分。
    score: f64,

    /// 累计得分里静态模型的部分（见 `Conversion::static_score`）。
    static_score: f64,

    /// 静态分数中由词频兜底贡献的部分。
    fallback_score: f64,

    /// 个人 n-gram 相对静态分数的累计差值。
    personal_delta: f64,

    /// 用户主动选择词的累计加分。
    selection_bonus: f64,

    /// 前驱在 `nodes[start]` 里的下标；`start == 0` 时无意义。
    back: usize,

    /// 是占位音节。
    placeholder: bool,

    /// 到此为止路径上模糊音 / 敲错变体的代价之和（已从 `score` 里扣掉，另记一份给调用方判断路径是不是原样）。
    penalty: f64,
}

/// 把音节序列转成最可能的词序列。`positions` 每个位置是若干写法（第一种是敲的，其余是模糊音 / 敲错变体），
/// `cost(位置, 命中的音节)` 是那个位置命中这种写法要扣的分（敲的原样 0），
/// `weight` 是用户选择次数，`personal` 是个人 n-gram 与插值参数（没有个人数据就传 [`Personal::NONE`]），`cache` 是格子候选的缓存
/// （调用方保证它与词库、`weight`、`personal`、`cost` 一致，这些一变就清）。
///
/// 简拼位置（`w x q`）按前缀取词：每个格子的候选会多得多，由语言模型在路径上分辨。
/// 全拼句子末尾的前缀太短时不算它（多半是没打完的音节）；前面已有简拼的句子里末尾单字母就是一个音节。
pub fn convert(
    dictionaries: &[&Dictionary],
    positions: &[Vec<SyllablePattern<'_>>],
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    weight: impl Fn(&str) -> u32,
    cost: impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
) -> Option<Conversion> {
    convert_with(
        dictionaries,
        positions,
        false,
        model,
        personal,
        weight,
        cost,
        cache,
    )
}

/// 同 [`convert`]，但全拼句子末尾的单字母也当一个音节读（`huo z…` → 或者）：
/// 给「整段拼音读法」与别的读法比分用，比分要两边覆盖同样多的字母。
pub fn convert_whole(
    dictionaries: &[&Dictionary],
    positions: &[Vec<SyllablePattern<'_>>],
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    weight: impl Fn(&str) -> u32,
    cost: impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
) -> Option<Conversion> {
    convert_with(
        dictionaries,
        positions,
        true,
        model,
        personal,
        weight,
        cost,
        cache,
    )
}

/// [`convert`] 与 [`convert_whole`] 的共同实现，`keep_partial` 选哪种；只要最优的一条。
#[allow(clippy::too_many_arguments)]
pub fn convert_with(
    dictionaries: &[&Dictionary],
    positions: &[Vec<SyllablePattern<'_>>],
    keep_partial: bool,
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    weight: impl Fn(&str) -> u32,
    cost: impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
) -> Option<Conversion> {
    convert_paths(
        dictionaries,
        positions,
        keep_partial,
        1,
        model,
        personal,
        weight,
        cost,
        cache,
    )
    .into_iter()
    .next()
}

/// 得分最高的前 `k` 条路径（最多束宽条，按得分降序，文本相同的只留一条）：给重打分用。
#[allow(clippy::too_many_arguments)]
pub fn convert_paths(
    dictionaries: &[&Dictionary],
    positions: &[Vec<SyllablePattern<'_>>],
    keep_partial: bool,
    k: usize,
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    weight: impl Fn(&str) -> u32,
    cost: impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
) -> Vec<Conversion> {
    convert_paths_inner(
        dictionaries,
        positions,
        keep_partial,
        k,
        model,
        personal,
        weight,
        cost,
        cache,
        None,
    )
}

/// 与 [`convert_paths`] 使用同一搜索过程，额外记录词格与每次束剪枝前的路径。
#[allow(clippy::too_many_arguments)]
pub fn convert_paths_diagnostic(
    dictionaries: &[&Dictionary],
    positions: &[Vec<SyllablePattern<'_>>],
    keep_partial: bool,
    k: usize,
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    weight: impl Fn(&str) -> u32,
    cost: impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
) -> (Vec<Conversion>, SearchDiagnostics) {
    let mut diagnostic = SearchDiagnostics::default();
    let paths = convert_paths_inner(
        dictionaries,
        positions,
        keep_partial,
        k,
        model,
        personal,
        weight,
        cost,
        cache,
        Some(&mut diagnostic),
    );
    (paths, diagnostic)
}

#[allow(clippy::too_many_arguments)]
fn convert_paths_inner(
    dictionaries: &[&Dictionary],
    positions: &[Vec<SyllablePattern<'_>>],
    keep_partial: bool,
    k: usize,
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    weight: impl Fn(&str) -> u32,
    cost: impl Fn(usize, &str) -> f64,
    cache: &mut SpanCache,
    mut diagnostic: Option<&mut SearchDiagnostics>,
) -> Vec<Conversion> {
    let Some((last, head)) = positions.split_last() else {
        return Vec::new();
    };
    let Some(&last) = last.first() else {
        return Vec::new();
    };
    let abbreviated_head = head.iter().any(|p| p.first().is_none_or(|t| !t.complete));
    let positions = if keep_partial
        || last.complete
        || abbreviated_head
        || last.text.len() >= MIN_PARTIAL_LETTERS
    {
        positions
    } else {
        head
    };
    let n = positions.len();
    if n == 0 || k == 0 {
        return Vec::new();
    }
    let total: f64 = dictionaries
        .iter()
        .map(|d| d.total_frequency() as f64)
        .sum::<f64>()
        .max(1.0);
    let log_total = total.ln();

    // nodes[i]：覆盖前 i 个音节、以某个词结尾的部分路径；nodes[0] 是虚拟起点
    let mut nodes: Vec<Vec<Node>> = (0..=n).map(|_| Vec::new()).collect();
    nodes[0].push(Node {
        start: 0,
        text: String::new(),
        route: String::new(),
        syllables: Vec::new(),
        score: 0.0,
        static_score: 0.0,
        fallback_score: 0.0,
        personal_delta: 0.0,
        selection_bonus: 0.0,
        back: 0,
        placeholder: false,
        penalty: 0.0,
    });
    for start in 0..n {
        prune(&mut nodes, start, k > 1, diagnostic.as_deref_mut());
        if nodes[start].is_empty() {
            continue;
        }
        let mut any = false;
        for end in start + 1..=n.min(start + MAX_WORD_SYLLABLES) {
            let span = &positions[start..end];
            let hits = cache.get_or_insert_with(SpanCache::key(span), || {
                span_candidates(dictionaries, span, start, personal, &weight, &cost)
            });
            if let Some(trace) = diagnostic.as_deref_mut() {
                trace.spans.push(SpanDiagnostic {
                    start,
                    end,
                    words: hits.iter().map(|hit| hit.text.clone()).collect(),
                });
            }
            if hits.is_empty() {
                continue;
            }
            any = true;
            for hit in hits.iter() {
                let bonus = weight_bonus(weight(&hit.text));
                let fallback = fallback_log_prob(hit.frequency, log_total);
                let predecessors = if k > 1 {
                    diverse_predecessors(&nodes, start, &hit.text, model, personal, fallback)
                } else {
                    vec![best_predecessor(
                        &nodes, start, &hit.text, model, personal, fallback,
                    )]
                };
                if let Some(trace) = diagnostic.as_deref_mut() {
                    for (index, predecessor) in nodes[start].iter().enumerate() {
                        let context = predecessor_context(&nodes, start, predecessor);
                        trace.transitions.push(TransitionDiagnostic {
                            end,
                            previous_text: backtrack(&nodes, start, index).text,
                            word: hit.text.clone(),
                            score: predecessor.score
                                + transition_log_prob(
                                    model, personal, context, &hit.text, fallback,
                                ),
                            selected: predecessors.iter().any(|(_, back)| *back == index),
                        });
                    }
                }
                for (score, back) in predecessors {
                    let previous = &nodes[start][back];
                    let penalty = previous.penalty + hit.penalty;
                    let static_step =
                        model.log_prob((start > 0).then_some(previous.text.as_str()), &hit.text);
                    let static_score = previous.static_score + static_step.unwrap_or(fallback);
                    let fallback_score = previous.fallback_score
                        + if static_step.is_none() { fallback } else { 0.0 };
                    let personal_delta = previous.personal_delta + score
                        - previous.score
                        - static_step.unwrap_or(fallback);
                    let selection_bonus = previous.selection_bonus + bonus;
                    let route = if k == 1 {
                        String::new()
                    } else if start == 0 {
                        hit.text.clone()
                    } else {
                        format!("{}\0{}", previous.route, hit.text)
                    };
                    nodes[end].push(Node {
                        start,
                        text: hit.text.clone(),
                        route,
                        syllables: hit.syllables.clone(),
                        score: score + bonus - hit.penalty,
                        static_score,
                        fallback_score,
                        personal_delta,
                        selection_bonus,
                        back,
                        placeholder: false,
                        penalty,
                    });
                }
            }
        }
        // 这个音节连单字都查不到：用音节本身占位，别让整句断掉
        if !any {
            let text = positions[start][0].text;
            let (score, back) = best_predecessor(
                &nodes,
                start,
                text,
                &NoModel,
                Personal::NONE,
                UNKNOWN_LOG_PROB,
            );
            let penalty = nodes[start][back].penalty;
            let static_score = nodes[start][back].static_score + UNKNOWN_LOG_PROB;
            let fallback_score = nodes[start][back].fallback_score + UNKNOWN_LOG_PROB;
            let personal_delta = nodes[start][back].personal_delta;
            let selection_bonus = nodes[start][back].selection_bonus;
            let route = if k == 1 {
                String::new()
            } else if start == 0 {
                text.to_owned()
            } else {
                format!("{}\0{}", nodes[start][back].route, text)
            };
            nodes[start + 1].push(Node {
                start,
                text: text.to_owned(),
                route,
                syllables: vec![text.to_owned()],
                score,
                static_score,
                fallback_score,
                personal_delta,
                selection_bonus,
                back,
                placeholder: true,
                penalty,
            });
        }
    }
    prune(&mut nodes, n, k > 1, diagnostic);
    let mut paths: Vec<Conversion> = Vec::with_capacity(k.min(nodes[n].len()));
    let mut indices: Vec<usize> = (0..nodes[n].len()).collect();
    if k > 1 {
        let routes: Vec<&str> = indices
            .iter()
            .map(|index| nodes[n][*index].route.as_str())
            .collect();
        let mut diverse = representative_indices(&routes, k.div_ceil(2));
        indices.retain(|index| !diverse.contains(index));
        diverse.extend(indices);
        indices = diverse;
    }
    for index in indices {
        if paths.len() >= k {
            break;
        }
        let conversion = backtrack(&nodes, n, index);
        if !paths.iter().any(|p| p.text == conversion.text) {
            paths.push(conversion);
        }
    }
    paths
}

/// 从 `nodes[position][index]` 回溯出整条路径。
fn backtrack(nodes: &[Vec<Node>], mut position: usize, mut index: usize) -> Conversion {
    let score = nodes[position][index].score;
    let static_score = nodes[position][index].static_score;
    let penalty = nodes[position][index].penalty;
    let mut words: Vec<SentenceWord> = Vec::new();
    while position > 0 {
        let node = &nodes[position][index];
        words.push(SentenceWord {
            text: node.text.clone(),
            syllables: node.syllables.clone(),
            placeholder: node.placeholder,
        });
        position = node.start;
        index = node.back;
    }
    words.reverse();
    let mut text = String::new();
    let mut syllables = Vec::new();
    for word in &words {
        text.push_str(&word.text);
        syllables.extend(word.syllables.iter().cloned());
    }
    Conversion {
        text,
        syllables,
        words,
        score,
        static_score,
        penalty,
    }
}

/// 占位音节不问语言模型。
struct NoModel;

impl LanguageModel for NoModel {
    fn log_prob(&self, _previous: Option<&str>, _word: &str) -> Option<f64> {
        None
    }
}

/// 一个格子里的候选词：所有词库的精确命中，按词频（加用户选择次数与个人出现次数，替代写法命中的按代价打折）取前几个。
/// 个人次数只在这里保证用户常用的同音词进得了格子，不进路径打分（那是 n-gram 的事）；
/// 打折让敲错变体命中的词只在原样命中不够多时才进格子，而常用词（关系）即使打折也留得住。
/// 格子里有简拼位置时命中的是一大片不同读音的词，多留一些让语言模型去挑。
fn span_candidates(
    dictionaries: &[&Dictionary],
    span: &[Vec<SyllablePattern<'_>>],
    start: usize,
    personal: Personal<'_>,
    weight: &impl Fn(&str) -> u32,
    cost: &impl Fn(usize, &str) -> f64,
) -> Vec<SpanWord> {
    let alternatives = span.iter().any(|p| p.len() > 1);
    let penalty_of = |m: &Match<'_>| {
        if !alternatives {
            return 0.0;
        }
        m.syllables()
            .enumerate()
            .map(|(index, syllable)| cost(start + index, syllable))
            .sum::<f64>()
    };
    // 得分先算好再排：单字母简拼的格子能命中几千条，比较器里每次查两张表会让排序占掉十几毫秒
    let mut scored: Vec<(f64, f64, Match<'_>)> = dictionaries
        .iter()
        .flat_map(|d| d.lookup_exact_alt(span))
        .map(|m| {
            let seen = weight(m.text) + personal.count(m.text);
            let penalty = penalty_of(&m);
            let score = f64::from(m.frequency) * (1.0 + f64::from(seen)) * (-penalty).exp();
            (score, penalty, m)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.dedup_by(|a, b| a.2.text == b.2.text);
    let abbreviated = span.iter().any(|p| p.iter().any(|t| !t.complete));
    scored.truncate(if abbreviated {
        ABBREVIATED_SPAN_CANDIDATES
    } else {
        SPAN_CANDIDATES
    });
    scored
        .into_iter()
        .map(|(_, penalty, hit)| SpanWord {
            text: hit.text.to_owned(),
            syllables: hit.syllables().map(str::to_owned).collect(),
            frequency: hit.frequency,
            penalty,
        })
        .collect()
}

/// 在 `nodes[start]` 的前驱里挑让 `word` 得分最高的那条，返回 (累计得分, 前驱下标)。
/// 转移概率先问静态模型（不认识就用词库兜底值），再与个人 n-gram 插值；前二词是前驱自己的前驱（回指）。
fn best_predecessor(
    nodes: &[Vec<Node>],
    start: usize,
    word: &str,
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    fallback: f64,
) -> (f64, usize) {
    let mut best = (f64::NEG_INFINITY, 0);
    for (index, previous) in nodes[start].iter().enumerate() {
        let context = predecessor_context(nodes, start, previous);
        let score = previous.score + transition_log_prob(model, personal, context, word, fallback);
        if score > best.0 {
            best = (score, index);
        }
    }
    best
}

/// 神经重排需要看到不同句首同音选择；同一个词格最多保留前三种句首的最佳前驱。
fn diverse_predecessors(
    nodes: &[Vec<Node>],
    start: usize,
    word: &str,
    model: &dyn LanguageModel,
    personal: Personal<'_>,
    fallback: f64,
) -> Vec<(f64, usize)> {
    let mut scored: Vec<(f64, usize)> = nodes[start]
        .iter()
        .enumerate()
        .map(|(index, previous)| {
            let context = predecessor_context(nodes, start, previous);
            (
                previous.score + transition_log_prob(model, personal, context, word, fallback),
                index,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let routes: Vec<&str> = scored
        .iter()
        .map(|(_, index)| nodes[start][*index].route.as_str())
        .collect();
    representative_indices(&routes, 3)
        .into_iter()
        .map(|index| scored[index])
        .collect()
}

fn predecessor_context<'a>(
    nodes: &'a [Vec<Node>],
    start: usize,
    previous: &'a Node,
) -> Context<'a> {
    if start == 0 {
        Context::START
    } else {
        Context {
            previous: Some(previous.text.as_str()),
            earlier: (previous.start > 0)
                .then(|| nodes[previous.start][previous.back].text.as_str()),
        }
    }
}

/// 按得分降序只留束宽条。
fn prune(
    nodes: &mut [Vec<Node>],
    position: usize,
    diverse: bool,
    diagnostic: Option<&mut SearchDiagnostics>,
) {
    nodes[position].sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut retained: Vec<usize> = Vec::with_capacity(BEAM_WIDTH);
    if diverse {
        let routes: Vec<&str> = nodes[position]
            .iter()
            .map(|node| node.route.as_str())
            .collect();
        retained = representative_indices(&routes, BEAM_WIDTH / 2);
    }
    for index in 0..nodes[position].len() {
        if retained.len() >= BEAM_WIDTH {
            break;
        }
        if !retained.contains(&index) {
            retained.push(index);
        }
    }
    if let Some(trace) = diagnostic {
        for index in 0..nodes[position].len() {
            let conversion = backtrack(nodes, position, index);
            let node = &nodes[position][index];
            trace.paths.push(PathDiagnostic {
                position,
                text: conversion.text,
                words: conversion.words.into_iter().map(|word| word.text).collect(),
                score: node.score,
                static_score: node.static_score,
                fallback_score: node.fallback_score,
                personal_delta: node.personal_delta,
                selection_bonus: node.selection_bonus,
                penalty: node.penalty,
                retained: retained.contains(&index),
            });
        }
    }
    if diverse {
        let old = std::mem::take(&mut nodes[position]);
        nodes[position] = old
            .into_iter()
            .enumerate()
            .filter_map(|(index, node)| retained.contains(&index).then_some(node))
            .collect();
        nodes[position].sort_by(|a, b| b.score.total_cmp(&a.score));
    } else {
        nodes[position].truncate(BEAM_WIDTH);
    }
}

#[cfg(test)]
mod tests;
