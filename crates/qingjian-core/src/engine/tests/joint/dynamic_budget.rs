//! 动态预算下的切分联合搜索：parser 排在第三条的切分也能靠词路径与语言模型证据胜出。
//!
//! 语料是受控词库造的：`anxianan` 的 parser 前两条切分（`an xian an`、`an xia nan`）
//! 在词格上只有单字路径，第三条 `an xi an an` 才有 `[安溪][安安]` 这条双词路径；
//! 三条切分都没有覆盖整段输入的完整词，所以「整段完整词」这条捷径帮不上忙，
//! 唯一能取胜的依据就是词路径 + 语言模型。

use super::*;
use crate::parser::Syllable;
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

/// parser 第 4 条切分才是对的，而且它落在旧的「按 parser 顺序消费探针预算」的射程之外：
/// 8 条有资格切分的前三条各要 36 / 36 / 44 个词格，旧预算 128 在第三条之后只剩 12，
/// 第 4 条（也是 44）直接被 `break` 掉，连探针都跑不到。
const FOURTH_SEGMENTATION_INPUT: &str = "xiliaxinixiaanxianan";

/// 只有第 4 条切分 `xi lia xi ni xia an xi a nan` 能在末尾拼出 `西阿南`（三音节跨度），
/// 其余切分的同一段是 `xian an` / `xia nan` / `xi an an`，都没有这个词。整段没有完整词，
/// 所以赢不了「整段完整词」这条捷径。
fn fourth_segmentation_dictionary() -> Dictionary {
    Dictionary::parse(concat!(
        "西\txi\t9000\n",
        "俩\tlia\t5000\n",
        "尼\tni\t6000\n",
        "下\txia\t9000\n",
        "安\tan\t9000\n",
        "南\tnan\t9000\n",
        "现\txian\t7000\n",
        "阿\ta\t1000\n",
        "西俩\txi lia\t8000\n",
        "西尼\txi ni\t8000\n",
        "下安\txia an\t8000\n",
        "西阿南\txi a nan\t9000\n",
        "现安\txian an\t1000\n",
        "下南\txia nan\t1000\n",
        "西安\txi an\t1000\n",
    ))
    .unwrap()
}

#[test]
fn fourth_parser_segmentation_still_gets_evidence_and_wins() {
    let segmentations = parser::segment(FOURTH_SEGMENTATION_INPUT).unwrap();
    assert_eq!(segmentations[0].to_string(), "xi lia xi ni xia an xian an");
    assert_eq!(segmentations[1].to_string(), "xi lia xi ni xia an xia nan");
    assert_eq!(segmentations[2].to_string(), "xi lia xi ni xia an xi an an");
    assert_eq!(
        segmentations[3].to_string(),
        "xi lia xi ni xia an xi a nan",
        "第 4 条才是对的"
    );
    // 没有整段完整词：赢不了捷径
    let mut engine = Engine::new(fourth_segmentation_dictionary());
    engine.set_input(FOURTH_SEGMENTATION_INPUT);
    let query = engine.query().unwrap();
    let stats = engine.last_joint_stats();
    assert_eq!(stats.eligible_segmentations, 8);
    // 第 4 条（下标 3）必须拿到探针证据：它排在 parser 名次的后半，旧的"按 parser 顺序消费预算"
    // 在它之前就把预算花光了。探针名额按结构轮转分配，每个结构第一轮各拿一个，所以它进得来。
    let probed = engine.last_probed_segmentations();
    assert!(
        probed.contains(&3),
        "第 4 条切分必须拿到探针证据，不能因为 parser 位置被饿死：{probed:?}"
    );
    assert_eq!(
        query.candidates.items[0].text, "西俩西尼下安西阿南",
        "第 4 条切分靠句尾那个只有它拼得出的三音节词胜出"
    );
    assert_eq!(
        query.segmentations[0].to_string(),
        "xi lia xi ni xia an xi a nan"
    );
}

/// 探针顺序按结构轮转：parser 名次不决定谁先拿到证据。
#[test]
fn probe_order_rotates_structures_instead_of_walking_parser_order() {
    use crate::engine::query::probe_order;
    // 八条切分八个不同结构（每条的两个尾巴音节长度都不一样），轮转后第一轮就是全体：
    // 结构不同的切分在各自组里都是第一个，"parser 排第 4 条"照样第一轮拿到证据。
    let segmentations: Vec<Segmentation> = (0..8)
        .map(|index| {
            let mut syllables = vec![Syllable::complete("xi"), Syllable::complete("lia")];
            for tail in 0..=index {
                syllables.push(Syllable::complete(if tail % 2 == 0 { "an" } else { "nan" }));
            }
            Segmentation { syllables }
        })
        .collect();
    let eligible: Vec<usize> = (0..segmentations.len()).collect();
    let order = probe_order(&eligible, &segmentations);
    assert_eq!(
        order, eligible,
        "每个结构各一条时第一轮就是全体，不看 parser 前缀"
    );

    // 同结构的两条轮流排在各自组里，不从整张表头开始数
    let grouped: Vec<Segmentation> = vec![
        vec!["xi", "lia", "an"],
        vec!["xi", "lia", "an"],
        vec!["xi", "li", "a"],
        vec!["xi", "li", "a"],
    ]
    .into_iter()
    .map(|syllables| Segmentation {
        syllables: syllables.into_iter().map(Syllable::complete).collect(),
    })
    .collect();
    let order = probe_order(&(0..4).collect::<Vec<usize>>(), &grouped);
    assert_eq!(order, vec![0, 2, 1, 3], "两个结构轮流：{order:?}");
}
