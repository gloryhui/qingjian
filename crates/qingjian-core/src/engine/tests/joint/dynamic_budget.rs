//! 动态预算下的切分联合搜索：parser 排在第三条的切分也能靠词路径与语言模型证据胜出。
//!
//! 语料是受控词库造的：`anxianan` 的 parser 前两条切分（`an xian an`、`an xia nan`）
//! 在词格上只有单字路径，第三条 `an xi an an` 才有 `[安溪][安安]` 这条双词路径；
//! 三条切分都没有覆盖整段输入的完整词，所以「整段完整词」这条捷径帮不上忙，
//! 唯一能取胜的依据就是词路径 + 语言模型。

use super::*;
use crate::sentence::{LanguageModel, Personal, SpanCache};

/// 只给第三条切分的词路径提供接续证据。
struct ThirdSegmentationBigram;

impl LanguageModel for ThirdSegmentationBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "安溪") => Some(-1.0),
            (Some("安溪"), "安安") => Some(-1.0),
            (None, "安") => Some(-3.0),
            (Some("安"), "现") => Some(-8.0),
            (Some("现"), "安") => Some(-8.0),
            (Some("安"), "下") => Some(-7.0),
            (Some("下"), "南") => Some(-7.0),
            _ => None,
        }
    }
}

const DICTIONARY: &str = "安\tan\t5000\n溪\txi\t3000\n安溪\tan xi\t6000\n安安\tan an\t4000\n现\txian\t9000\n下\txia\t9000\n南\tnan\t9000\n";

fn engine() -> Engine {
    Engine::new(Dictionary::parse(DICTIONARY).unwrap())
        .with_language_model(Box::new(ThirdSegmentationBigram))
}

#[test]
fn parser_keeps_three_complete_segmentations_with_the_target_last() {
    let segmentations = parser::segment("anxianan").unwrap();
    assert_eq!(segmentations[0].to_string(), "an xian an");
    assert_eq!(segmentations[1].to_string(), "an xia nan");
    assert_eq!(
        segmentations[2].to_string(),
        "an xi an an",
        "第三条才是对的"
    );
    assert!(
        segmentations[..3]
            .iter()
            .all(|s| s.incomplete_count() == 0 && s.syllables.len() >= 2),
        "三条都完整，旧实现的固定两条预算正好把它挡在外面"
    );
}

/// 前两条切分的词格上根本走不出目标句：不搜索第三条就永远拿不到它。
#[test]
fn the_target_text_is_unreachable_on_the_first_two_segmentations() {
    let segmentations = parser::segment("anxianan").unwrap();
    let dictionary = Dictionary::parse(DICTIONARY).unwrap();
    for index in [0, 1] {
        let positions: Vec<_> = segmentations[index]
            .patterns()
            .into_iter()
            .map(|pattern| vec![pattern])
            .collect();
        let paths = sentence::convert_paths(
            &[&dictionary],
            &positions,
            false,
            8,
            &ThirdSegmentationBigram,
            Personal::NONE,
            |_| 0,
            |_, _| 0.0,
            &mut SpanCache::default(),
        );
        assert!(
            paths.iter().all(|path| path.text != "安溪安安"),
            "切分 #{index} 不该造得出目标句：{:?}",
            paths.iter().map(|path| &path.text).collect::<Vec<_>>()
        );
    }
}

#[test]
fn third_segmentation_wins_the_joint_search() {
    let mut engine = engine();
    engine.set_input("anxianan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "安溪安安");
    assert_eq!(
        query.segmentations[0].to_string(),
        "an xi an an",
        "顶部拼音跟随获胜切分"
    );
    assert_eq!(query.marked_text(), "an'xi'an'an");
}

#[test]
fn third_segmentation_wins_with_neural_rescoring_enabled() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = engine().with_sentence_scorer(
        Box::new(PrefersWholeSentence {
            target: "安溪安安",
            seen: Arc::clone(&seen),
        }),
        Some(1.0),
        None,
        None,
    );
    engine.set_input("anxianan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "安溪安安");
    assert_eq!(query.marked_text(), "an'xi'an'an");
    assert!(
        seen.lock().unwrap().iter().any(|text| text == "安溪安安"),
        "正确切分必须进入重排池，不能因为预算根本没被搜索"
    );
}

/// 神经模型压另一条时也不该让正确切分连候选池都进不去。
#[test]
fn third_segmentation_still_reaches_the_pool_when_the_model_disagrees() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = engine().with_sentence_scorer(
        Box::new(PrefersWholeSentence {
            target: "安下南",
            seen: Arc::clone(&seen),
        }),
        Some(1.0),
        None,
        None,
    );
    engine.set_input("anxianan");
    let query = engine.query().unwrap();
    assert!(
        seen.lock().unwrap().iter().any(|text| text == "安溪安安"),
        "正确切分必须进入 scorer 视野"
    );
    assert_eq!(query.candidates.items[0].text, "安下南");
}

#[test]
fn explicit_apostrophe_remains_a_hard_boundary() {
    let segmentations = parser::segment("an'xi'an'an").unwrap();
    assert_eq!(segmentations.len(), 1);
    assert_eq!(segmentations[0].to_string(), "an xi an an");
    let mut engine = engine();
    engine.set_input("an'xi'an'an");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "安溪安安");
    assert_eq!(query.marked_text(), "an'xi'an'an");
}

/// 个人 n-gram 让本来落后的切分翻身：探针也走同一条打分链，所以预算不会被静态分吞掉。
#[test]
fn personal_evidence_lifts_a_later_segmentation_into_budget() {
    let dictionary = Dictionary::parse(DICTIONARY).unwrap();
    let mut learner = WordLearner::default();
    for _ in 0..6 {
        learner.record_transition(sentence::Context::START, "安溪", 2);
        learner.record_transition(sentence::Context::after("安溪"), "安安", 2);
    }
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(StaticOnlyFirstTwo))
        .with_learner(Box::new(learner));
    engine.set_input("anxianan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "安溪安安");
    assert_eq!(query.segmentations[0].to_string(), "an xi an an");
}

/// 静态模型只认前两条切分的路径：没有个人证据时第三条赢不了，有了才赢得回来。
struct StaticOnlyFirstTwo;

impl LanguageModel for StaticOnlyFirstTwo {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "安") => Some(-1.0),
            (Some("安"), "下") => Some(-1.0),
            (Some("下"), "南") => Some(-1.0),
            (None, "安溪") => Some(-9.0),
            (Some("安溪"), "安安") => Some(-9.0),
            _ => None,
        }
    }
}

#[test]
fn without_personal_evidence_the_static_model_still_decides() {
    let mut engine = Engine::new(Dictionary::parse(DICTIONARY).unwrap())
        .with_language_model(Box::new(StaticOnlyFirstTwo));
    engine.set_input("anxianan");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "安下南");
    assert_eq!(query.segmentations[0].to_string(), "an xia nan");
}

#[test]
fn joint_stats_report_the_budgeted_search() {
    let mut engine = engine();
    engine.set_input("anxianan");
    let _ = engine.query().unwrap();
    let stats = engine.last_joint_stats();
    assert_eq!(stats.parser_segmentations, 8);
    assert_eq!(stats.eligible_segmentations, 4);
    assert!(stats.probed_segmentations >= 3, "{stats:?}");
    assert!(stats.searched_segmentations <= 4, "{stats:?}");
    assert!(stats.spans > 0, "{stats:?}");
    assert!(stats.viterbi_paths > 0, "{stats:?}");
}

/// 双拼解出的拼音没有切分歧义，多切分预算这条路不套用；解码结果照常走整句与词级查询。
#[test]
fn shuangpin_decodes_to_a_single_segmentation_and_keeps_the_word() {
    let dictionary =
        Dictionary::parse("蛋糕\tdan gao\t9000\n当\tdang\t1000\n奥\tao\t1000\n").unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_shuangpin(Some(crate::shuangpin::Scheme::Xiaohe));
    engine.set_input("djgc");
    assert_eq!(engine.decode("djgc").unwrap().pinyin(), "dan'gao");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "蛋糕");
    assert_eq!(engine.last_joint_stats().parser_segmentations, 1);
}
