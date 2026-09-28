//! 神经重打分：整句转换的前几条路径交给字级模型（[`SentenceScorer`]）再排一次。
//!
//! 打分有两种接法：同步的（[`Engine::with_sentence_scorer`]，查询里当场打，CLI 评测用）和异步的
//! （[`Engine::with_async_sentence_scorer`]，后台线程；壳里用）。两种都经过一张「前文 + 文本 → 神经分」的缓存
//! （[`NeuralCache`]）：同步时缺的分当场补进去，异步时缺的先记下来，壳在用户停顿后调 [`Engine::request_rescoring`]
//! 一次送去后台，[`Engine::poll_rescoring`] 收到结果后再查一次，这时全部路径的分都在缓存里，排序自然换成重排后的。
//! 按键回调永远不等模型：先按词级模型出候选，模型的意见晚几十毫秒到。

mod cache;
mod worker;

#[cfg(test)]
mod tests;

use super::*;
use std::collections::HashSet;

pub(crate) use cache::NeuralCache;
pub(crate) use worker::RescoreWorker;

impl Engine {
    /// 文本只占一个重排名额；同文本的各条词路径都要参加分数替换，个人证据和代价可能不同。
    pub(super) fn retain_neural_eligible_by_text<T>(
        &self,
        paths: &mut Vec<T>,
        text: impl Fn(&T) -> &str,
        score: impl Fn(&T) -> f64,
    ) {
        if !self.has_sentence_scorer() || paths.len() < 2 {
            return;
        }
        let best = paths
            .iter()
            .map(&score)
            .max_by(f64::total_cmp)
            .expect("nonempty path pool");
        let floor = best - self.neural_margin;
        let eligible: HashSet<String> = paths
            .iter()
            .filter(|path| score(path) >= floor)
            .map(|path| text(path).to_owned())
            .collect();
        paths.retain(|path| eligible.contains(text(path)));
    }
    /// 接了重打分器（同步或异步）。
    pub fn has_sentence_scorer(&self) -> bool {
        self.sentence_scorer.is_some()
            || self.rescorer.as_ref().is_some_and(RescoreWorker::is_alive)
    }

    /// 给模型看的前文：壳给了应用里的光标前文就用它（[`Self::set_rescoring_context`]），
    /// 否则用本会话最近上屏的字符；长度按 `neural_context` 截。
    pub(super) fn rescoring_context(&self) -> String {
        if self.neural_context == 0 {
            return String::new();
        }
        match &self.rescoring_before {
            Some(before) => take_last_chars(before, self.neural_context),
            None => self.history.recent(self.neural_context).to_owned(),
        }
    }

    /// 壳告知应用里光标前的文本（每次查询前给；应用给不出就 `None`，退回本会话历史）。
    pub fn set_rescoring_context(&mut self, before: Option<String>) {
        self.rescoring_before = before;
    }

    /// 把几条整句路径按「路径分 + λ·(神经分 − 静态分)」重排。缓存里缺分的：同步打分器当场补，异步的先记下等壳来取；
    /// 有任何一条没分就不动顺序（半截重排比不重排还糟）。
    pub(super) fn rescore_paths(&self, paths: &mut [Conversion]) {
        if self.rescore_path_scores(paths) {
            paths.sort_by(|a, b| b.score.total_cmp(&a.score));
        }
    }

    /// 更新分数但保留路径与调用方索引的对应关系。
    pub(super) fn rescore_path_scores(&self, paths: &mut [Conversion]) -> bool {
        if paths.len() < 2 || !self.has_sentence_scorer() {
            return false;
        }
        let context = self.rescoring_context();
        let mut cache = self.neural_cache.borrow_mut();
        cache.ensure_context(&context);
        let mut missing: Vec<String> = Vec::new();
        for path in paths.iter() {
            if cache.get(&path.text).is_none() && !missing.contains(&path.text) {
                missing.push(path.text.clone());
            }
        }
        if !missing.is_empty() {
            match &self.sentence_scorer {
                Some(scorer) => {
                    let texts: Vec<&str> = missing.iter().map(String::as_str).collect();
                    let scores = scorer.score(&context, &texts);
                    if scores.len() != texts.len() {
                        return false;
                    }
                    for (text, score) in texts.iter().zip(scores) {
                        cache.insert(text, score);
                    }
                }
                None => {
                    for text in &missing {
                        cache.want(text);
                    }
                    return false;
                }
            }
        }
        let lambda = self.neural_weight;
        for path in paths.iter_mut() {
            let neural = cache.get(&path.text).expect("filled above");
            path.score += lambda * (neural - path.static_score);
        }
        self.last_rescored.set(true);
        true
    }

    /// 最近一次查询里有整句路径还没拿到神经分：壳该在用户停顿后调 [`Self::request_rescoring`]。
    pub fn rescoring_pending(&self) -> bool {
        self.rescorer.is_some() && self.neural_cache.borrow().has_wanted()
    }

    /// 把攒着的文本送去后台打分。没接异步打分器或没什么要打的返回 `false`。
    pub fn request_rescoring(&mut self) -> bool {
        let Some(worker) = &self.rescorer else {
            return false;
        };
        let mut cache = self.neural_cache.borrow_mut();
        let wanted = cache.take_wanted();
        if wanted.is_empty() {
            return false;
        }
        tracing::debug!(texts = wanted.len(), "神经重打分请求");
        worker.submit(cache.context().to_owned(), wanted);
        true
    }

    /// 收后台打好的分。有新分进了缓存返回 `true`，壳该重新 [`Self::query`] 一次；前文已经变了的结果丢掉。
    pub fn poll_rescoring(&mut self) -> bool {
        let Some(worker) = &self.rescorer else {
            return false;
        };
        let mut updated = false;
        while let Some(scored) = worker.poll() {
            let mut cache = self.neural_cache.borrow_mut();
            if scored.context != cache.context() || scored.scores.len() != scored.texts.len() {
                continue;
            }
            for (text, score) in scored.texts.iter().zip(scored.scores) {
                cache.insert(text, score);
            }
            updated = true;
        }
        updated
    }
}
