//! 联合切分与完整句重排的回归语料；首选和顶部拼音一起检查。

use std::sync::{Arc, Mutex};

use super::*;
use crate::sentence::{LanguageModel, SentenceScorer};

mod complete_word;

fn real_dictionary() -> Dictionary {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/lexicon/dict.tsv");
    Dictionary::from_path(path).unwrap()
}

#[test]
fn segmentation_and_context_regression_corpus() {
    let mut engine = Engine::new(real_dictionary());
    for (input, expected) in [
        ("keneng", "可能"),
        ("ke'neng", "可能"),
        ("wan'dou'bu'xiang'wan", "玩都不想玩"),
        ("wan'dou'ke'yi'wan", "玩都可以玩"),
        ("wan'dou'bu'yao'wan", "玩都不要玩"),
        ("ping'guo'bu'xiang'chi", "苹果不想吃"),
        ("jin'tian'bu'xiang'wan", "今天不想玩"),
        ("kan'dou'bu'xiang'kan", "看都不想看"),
        ("mai'dou'bu'xiang'mai", "买都不想买"),
    ] {
        engine.set_input(input);
        let query = engine.query().unwrap();
        assert_eq!(query.candidates.items[0].text, expected, "{input}");
        if input == "keneng" || input == "ke'neng" {
            assert_eq!(query.marked_text(), "ke'neng");
        }
    }
    assert_eq!(parser::segment("ke'neng").unwrap().len(), 1);
    // 基础词库没有“坐诊”：这是 DICTIONARY_GAP，解码器不应凭空造词。
    engine.set_input("zuo'zhen");
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .all(|candidate| candidate.text != "坐诊")
    );
}

struct BiasedBigram;

impl LanguageModel for BiasedBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "晚") => Some(-1.0),
            (None, "万") => Some(-2.0),
            (None, "玩") => Some(-3.0),
            (_, "都" | "不想" | "玩") => Some(-1.0),
            _ => None,
        }
    }
}

struct PrefersWholeSentence {
    target: &'static str,

    seen: Arc<Mutex<Vec<String>>>,
}

impl SentenceScorer for PrefersWholeSentence {
    fn score(&self, _context: &str, texts: &[&str]) -> Vec<f64> {
        self.seen
            .lock()
            .unwrap()
            .extend(texts.iter().map(|text| (*text).to_owned()));
        texts
            .iter()
            .map(|text| if *text == self.target { -1.0 } else { -20.0 })
            .collect()
    }
}

#[test]
fn full_sentence_evidence_can_reverse_an_early_homophone_choice() {
    let dictionary = Dictionary::parse(
        "晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let mut static_only = Engine::new(dictionary).with_language_model(Box::new(BiasedBigram));
    static_only.set_input("wan'dou'bu'xiang'wan");
    assert_eq!(
        static_only.query().unwrap().candidates.items[0].text,
        "晚都不想玩"
    );

    let dictionary = Dictionary::parse(
        "晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let scorer = PrefersWholeSentence {
        target: "玩都不想玩",
        seen: Arc::clone(&seen),
    };
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(BiasedBigram))
        .with_sentence_scorer(Box::new(scorer), Some(1.0), None, None);
    engine.set_input("wan'dou'bu'xiang'wan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "玩都不想玩");
    assert!(seen.lock().unwrap().iter().any(|text| text == "玩都不想玩"));
}

struct DuplicatePathScorer;

struct DuplicateTextBigram;

impl LanguageModel for DuplicateTextBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "我") => Some(-0.1),
            (Some("我"), "研究") => Some(-0.1),
            (Some("我"), "研究生") => Some(-2.9),
            (Some("研究"), "声") => Some(-0.8),
            (Some("研究"), "生") => Some(-1.8),
            _ => None,
        }
    }
}

impl SentenceScorer for DuplicatePathScorer {
    fn score(&self, _context: &str, texts: &[&str]) -> Vec<f64> {
        texts
            .iter()
            .map(|text| match *text {
                "我研究声" => -2.5,
                "我研究生" => -1.0,
                _ => -20.0,
            })
            .collect()
    }
}

#[test]
fn neural_rescore_keeps_the_best_same_text_path_after_terminal_diversity() {
    let dictionary = Dictionary::parse(
        "我\two\t1000\n研究\tyan jiu\t1000\n研究生\tyan jiu sheng\t1000\n生\tsheng\t1000\n声\tsheng\t1000\n",
    )
    .unwrap();
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(DuplicateTextBigram))
        .with_sentence_scorer(Box::new(DuplicatePathScorer), Some(0.5), Some(4.0), None);
    engine.set_input("wo'yan'jiu'sheng");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "我研究生");
}

#[test]
fn multi_syllable_homophone_path_reaches_whole_sentence_scorer() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let scorer = PrefersWholeSentence {
        target: "医生不想来",
        seen: Arc::clone(&seen),
    };
    let mut engine = Engine::new(real_dictionary()).with_sentence_scorer(
        Box::new(scorer),
        Some(1.0),
        None,
        None,
    );
    engine.set_input("yi'sheng'bu'xiang'lai");
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "医生不想来"
    );
    assert!(seen.lock().unwrap().iter().any(|text| text == "医生不想来"));
}

mod async_rescoring;
mod diversity;
mod ranking;
