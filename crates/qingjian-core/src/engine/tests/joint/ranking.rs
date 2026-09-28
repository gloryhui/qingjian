//! 跨切分词级排序、神经门槛与顶部拼音回归。

use super::*;
use std::collections::HashMap;

#[test]
fn learned_complete_word_keeps_its_choice_rank_across_segmentations() {
    let dictionary = "方案\tfang an\t1000\n反感\tfan gan\t100000\n";
    for neural in [false, true] {
        let mut counts = HashMap::new();
        counts.insert("方案".to_owned(), 2);
        counts.insert("fangan\t方案".to_owned(), 2);
        let mut engine = Engine::new(Dictionary::parse(dictionary).unwrap())
            .with_learner(Box::new(CountingLearner(counts)));
        if neural {
            engine = engine.with_sentence_scorer(
                Box::new(PrefersWholeSentence {
                    target: "反感",
                    seen: Arc::new(Mutex::new(Vec::new())),
                }),
                Some(1.0),
                None,
                None,
            );
        }
        engine.set_input("fangan");
        let query = engine.query().unwrap();
        assert_eq!(query.candidates.items[0].text, "方案", "neural={neural}");
        assert_eq!(query.marked_text(), "fang'an", "neural={neural}");
    }
}

#[test]
fn learned_first_word_sets_the_displayed_segmentation() {
    let dictionary = Dictionary::parse("方案\tfang an\t1000\n反感\tfan gan\t100000\n").unwrap();
    let mut counts = HashMap::new();
    counts.insert("反感".to_owned(), 2);
    counts.insert("fangan\t反感".to_owned(), 2);
    let mut engine = Engine::new(dictionary).with_learner(Box::new(CountingLearner(counts)));
    engine.set_input("fangan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "反感");
    assert_eq!(query.marked_text(), "fan'gan");
}

pub(super) struct WideGapBigram;

impl LanguageModel for WideGapBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "晚") => Some(-1.0),
            (None, "万") => Some(-2.0),
            (None, "玩") => Some(-11.0),
            (_, "都" | "不想" | "玩") => Some(-1.0),
            _ => None,
        }
    }
}

#[test]
fn joint_margin_excludes_weak_protected_path_before_neural_scoring() {
    let dictionary = Dictionary::parse(
        "晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(WideGapBigram))
        .with_sentence_scorer(
            Box::new(PrefersWholeSentence {
                target: "玩都不想玩",
                seen: Arc::clone(&seen),
            }),
            Some(1.0),
            Some(4.0),
            None,
        );
    engine.set_input("wan'dou'bu'xiang'wan");
    let query = engine.query().unwrap();
    assert_ne!(query.candidates.items[0].text, "玩都不想玩");
    assert!(!seen.lock().unwrap().iter().any(|text| text == "玩都不想玩"));
}

struct SplitWordBigram;

impl LanguageModel for SplitWordBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "可") | (Some("可"), "能") => Some(-1.0),
            (None, "可能") => Some(-5.0),
            (None, "肯") | (Some("肯"), "鞥") => Some(-9.0),
            _ => None,
        }
    }
}

#[test]
fn duplicate_sentence_text_still_updates_winning_preedit() {
    let dictionary = Dictionary::parse(
        "可能\tke neng\t100000\n可\tke\t9000\n能\tneng\t9000\n肯\tken\t2000\n鞥\teng\t2000\n",
    )
    .unwrap();
    let mut engine = Engine::new(dictionary).with_language_model(Box::new(SplitWordBigram));
    engine.set_input("keneng");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "可能");
    assert_eq!(
        query
            .candidates
            .items
            .iter()
            .filter(|c| c.text == "可能")
            .count(),
        1
    );
    assert_eq!(query.marked_text(), "ke'neng");
}

struct TwoSegmentationGap;

impl LanguageModel for TwoSegmentationGap {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "方案") => Some(-5.0),
            (None, "反感") => Some(-11.0),
            _ => None,
        }
    }
}

#[test]
fn margin_uses_the_best_score_across_both_segmentations() {
    let dictionary = Dictionary::parse("方案\tfang an\t1000\n反感\tfan gan\t100000\n").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(TwoSegmentationGap))
        .with_sentence_scorer(
            Box::new(PrefersWholeSentence {
                target: "反感",
                seen: Arc::clone(&seen),
            }),
            Some(1.0),
            Some(4.0),
            None,
        );
    engine.set_input("fangan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "方案");
    assert_eq!(query.marked_text(), "fang'an");
    assert!(!seen.lock().unwrap().iter().any(|text| text == "反感"));
}
