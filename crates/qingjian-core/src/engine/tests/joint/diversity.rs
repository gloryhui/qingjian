//! 共同前缀和双切分路径代表回归。

use super::*;

struct PrefixBigram;

impl LanguageModel for PrefixBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "我") => Some(-1.0),
            (Some("我"), "晚") => Some(-1.0),
            (Some("我"), "万") => Some(-2.0),
            (Some("我"), "玩") => Some(-3.0),
            (_, "都" | "不想" | "玩") => Some(-1.0),
            _ => None,
        }
    }
}

#[test]
fn shared_prefix_homophone_survives_until_whole_sentence_scorer() {
    let dictionary = Dictionary::parse(
        "我\two\t10000\n晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(PrefixBigram))
        .with_sentence_scorer(
            Box::new(PrefersWholeSentence {
                target: "我玩都不想玩",
                seen: Arc::clone(&seen),
            }),
            Some(1.0),
            None,
            None,
        );
    engine.set_input("wo'wan'dou'bu'xiang'wan");
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "我玩都不想玩"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|text| text == "我玩都不想玩")
    );
}

struct DeepPrefixBigram;

impl LanguageModel for DeepPrefixBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "我") | (Some("我"), "今天") => Some(-1.0),
            (Some("今天"), "卖") => Some(-1.0),
            (Some("今天"), "买") => Some(-3.0),
            (_, "都" | "不想" | "买") => Some(-1.0),
            _ => None,
        }
    }
}

#[test]
fn deeper_common_prefix_keeps_a_different_homophone() {
    let dictionary = Dictionary::parse(
        "我\two\t10000\n今天\tjin tian\t9000\n卖\tmai\t8000\n买\tmai\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(DeepPrefixBigram))
        .with_sentence_scorer(
            Box::new(PrefersWholeSentence {
                target: "我今天买都不想买",
                seen: Arc::clone(&seen),
            }),
            Some(1.0),
            None,
            None,
        );
    engine.set_input("wo'jin'tian'mai'dou'bu'xiang'mai");
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "我今天买都不想买"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|text| text == "我今天买都不想买")
    );
}

#[test]
fn two_segmentations_keep_a_local_homophone_representative_for_scorer() {
    let dictionary = Dictionary::parse(
        "方案\tfang an\t10000\n方\tfang\t9500\n放\tfang\t9000\n访\tfang\t8500\n案\tan\t10000\n反感\tfan gan\t9500\n烦感\tfan gan\t9000\n都\tdou\t10000\n不想\tbu xiang\t10000\n玩\twan\t10000\n",
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(dictionary).with_sentence_scorer(
        Box::new(PrefersWholeSentence {
            target: "放案都不想玩",
            seen: Arc::clone(&seen),
        }),
        Some(1.0),
        None,
        None,
    );
    engine.set_input("fangan'dou'bu'xiang'wan");
    let query = engine.query().unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|text| text == "放案都不想玩")
    );
    assert_eq!(query.candidates.items[0].text, "放案都不想玩");
}
