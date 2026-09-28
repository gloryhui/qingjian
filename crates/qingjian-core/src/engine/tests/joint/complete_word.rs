//! 完整词、个人学习与整句神经重排之间的优先级边界。

use super::*;
use std::time::{Duration, Instant};

struct StaticSplitBigram;

impl LanguageModel for StaticSplitBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "当") => Some(-1.0),
            (Some("当"), "奥") => Some(-1.0),
            (None, "蛋糕") => Some(-5.0),
            _ => None,
        }
    }
}

fn complete_word_engine() -> Engine {
    let dictionary =
        Dictionary::parse("蛋糕\tdan gao\t100000\n当\tdang\t1000\n奥\tao\t1000\n").unwrap();
    Engine::new(dictionary).with_language_model(Box::new(StaticSplitBigram))
}

fn prefers_split(seen: Arc<Mutex<Vec<String>>>) -> PrefersWholeSentence {
    PrefersWholeSentence {
        target: "当奥",
        seen,
    }
}

#[test]
fn unlearned_static_split_does_not_take_first_place_from_a_complete_word() {
    let mut engine = complete_word_engine();
    engine.set_input("dangao");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "蛋糕");
    assert!(
        query
            .candidates
            .items
            .iter()
            .all(|candidate| candidate.text != "当奥")
    );
}

#[test]
fn shuangpin_complete_word_protection_uses_decoded_reading() {
    let mut engine = complete_word_engine();
    engine.set_shuangpin(Some(crate::shuangpin::Scheme::Xiaohe));
    engine.set_input("djgc");
    assert_eq!(engine.decode("djgc").unwrap().pinyin(), "dan'gao");

    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "蛋糕");
    assert!(
        query
            .candidates
            .items
            .iter()
            .all(|candidate| candidate.text != "当奥")
    );
}

#[test]
fn synchronous_neural_score_does_not_override_a_complete_word() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = complete_word_engine().with_sentence_scorer(
        Box::new(prefers_split(Arc::clone(&seen))),
        Some(1.0),
        Some(100.0),
        None,
    );
    engine.set_input("dangao");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "蛋糕");
    assert!(seen.lock().unwrap().iter().any(|text| text == "当奥"));
}

#[test]
fn async_neural_score_does_not_override_a_complete_word() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = complete_word_engine().with_async_sentence_scorer(
        Box::new(prefers_split(Arc::clone(&seen))),
        Some(1.0),
        Some(100.0),
        None,
    );
    engine.set_input("dangao");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "蛋糕");
    assert!(engine.rescoring_pending());
    assert!(engine.request_rescoring());

    let started = Instant::now();
    while !engine.poll_rescoring() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "后台重排没有返回"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "蛋糕");
    assert!(seen.lock().unwrap().iter().any(|text| text == "当奥"));
}

#[test]
fn neural_context_can_still_choose_a_split_without_an_exact_full_word() {
    let dictionary = Dictionary::parse("当\tdang\t1000\n奥\tao\t1000\n").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(StaticSplitBigram))
        .with_sentence_scorer(
            Box::new(prefers_split(Arc::clone(&seen))),
            Some(1.0),
            Some(100.0),
            None,
        );
    engine.set_input("dangao");

    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "当奥");
    assert!(seen.lock().unwrap().iter().any(|text| text == "当奥"));
}
