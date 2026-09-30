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
/// 这条输入有 8 条完整切分，命中「probe 名额只有 4 个」的场景。
const MANY_SEGMENTATIONS_INPUT: &str = "xiliaxinixiaanxianan";

/// 只有第 6 条切分 `xi li a xi ni xia an xian an` 能在开头用上三音节词 `西里阿`，
/// 于是它用 4 个词盖住 9 个音节（平均词长 2.25），其余切分要么用 4 个词盖 8 个音节（2.0）、
/// 要么只能一个字一个字盖（1.5）。整段没有完整词，赢不了「整段完整词」这条捷径。
fn many_segmentations_dictionary() -> Dictionary {
    Dictionary::parse(concat!(
        "西\txi\t9000\n",
        "里\tli\t9000\n",
        "阿\ta\t9000\n",
        "俩\tlia\t9000\n",
        "尼\tni\t9000\n",
        "下\txia\t9000\n",
        "安\tan\t9000\n",
        "南\tnan\t9000\n",
        "现\txian\t9000\n",
        "西里阿\txi li a\t20000\n",
        "西俩\txi lia\t100\n",
        "西尼\txi ni\t100\n",
        "下安\txia an\t100\n",
        "现安\txian an\t9000\n",
        "下南\txia nan\t100\n",
    ))
    .unwrap()
}

/// RED→GREEN：正确的切分排在 parser 第 7 条（整段探针名额只有 4 个），
/// 靠**廉价证据**抢进整段探针，再靠只有它拼得出的三音节词胜出。
///
/// 词级候选里只有 `西里阿`（第 7、8 条切分的前缀）和 `西俩`（第 1…6 条切分的前缀）两个多音节词；
/// 第 7、8 条因此能用 3 + 1 个词盖住 9 个音节里的 4 个，平均词长 2.0，比前面各条的 1.5 / 1.33 都长。
///
/// 旧实现按 parser 顺序拿探针名额（前 4 条是 `#0..#3`），第 7 条连整段探针都跑不到，
/// 测试在旧实现下首选 `西俩西尼下安现安`，不是答案。
#[test]
fn a_late_segmentation_wins_a_probe_slot_on_cheap_evidence() {
    let segmentations = parser::segment(MANY_SEGMENTATIONS_INPUT).unwrap();
    assert_eq!(segmentations[0].to_string(), "xi lia xi ni xia an xian an");
    assert_eq!(
        segmentations[6].to_string(),
        "xi li a xi ni xia an xian an",
        "第 7 条才是对的"
    );

    let mut engine = Engine::new(many_segmentations_dictionary());
    engine.set_input(MANY_SEGMENTATIONS_INPUT);
    let query = engine.query().unwrap();

    // 廉价证据覆盖了全部有资格切分，而且第 7 条不弱于任何排在它前面的切分
    let evidence = engine.last_cheap_evidence();
    assert_eq!(evidence.len(), 8, "每条有资格的切分都要有廉价证据");
    for slot in 0..6 {
        assert!(
            evidence[6].1 >= evidence[slot].1,
            "第 7 条的词法证据不能弱于第 {} 条：{evidence:?}",
            slot + 1
        );
    }

    // 它因此拿到整段探针名额，并且排在 parser 靠前的那些前面
    let probed = engine.last_probed_segmentations();
    assert!(
        probed.contains(&6),
        "第 7 条必须靠廉价证据抢进整段探针：{probed:?}"
    );
    assert_eq!(probed[0], 6, "廉价证据最强的先拿名额：{probed:?}");
    assert_eq!(query.candidates.items[0].text, "西里阿西尼下安现安");
    assert_eq!(
        query.segmentations[0].to_string(),
        "xi li a xi ni xia an xian an"
    );
}

/// RED→GREEN：两条切分的 `StructureKey` 完全相同（都是 3 个完整音节），
/// 后一条的词法证据更强时不能被前一条永久代表掉。
///
/// 词级候选里 `安下`（第 2 条切分的前缀 `an xia`）存在、`安现`（第 1 条切分的前缀 `an xian`）不存在：
/// 第 2 条能用 2 个词盖住 3 个音节（平均词长 2.0），第 1 条只能一个字一个字盖（1.0）。
#[test]
fn same_structure_key_does_not_make_two_segmentations_semantically_equal() {
    use crate::engine::query::select_probe_candidates;
    // 纯选择逻辑：名额只够一个时，同结构组里证据更强的那个才是代表
    let flat: Vec<Segmentation> = [["fang", "an"], ["fan", "gan"]]
        .into_iter()
        .map(|syllables| Segmentation {
            syllables: syllables.into_iter().map(Syllable::complete).collect(),
        })
        .collect();
    let chosen = select_probe_candidates(&[0, 1], &flat, &[(1.0, 1.0), (2.0, 1.0)], 1);
    assert_eq!(chosen, vec![1], "同结构组要按证据选代表，不看 parser 名次");
    let chosen = select_probe_candidates(&[0, 1], &flat, &[(1.0, 1.0), (2.0, 1.0)], 2);
    assert_eq!(chosen, vec![1, 0], "名额够时同组其他成员也不被代表掉");

    // 真实查询：`#0 an xian an` 与 `#1 an xia nan` 同结构签名 (3, 0)
    let dictionary = Dictionary::parse(concat!(
        "安\tan\t9000\n",
        "现\txian\t9000\n",
        "下\txia\t9000\n",
        "南\tnan\t9000\n",
        "西\txi\t9000\n",
        "阿\ta\t9000\n",
        "安下\tan xia\t20000\n",
        "安西\tan xi\t9000\n",
    ))
    .unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_input("anxianan");
    let query = engine.query().unwrap();
    let evidence = engine.last_cheap_evidence();
    let first = evidence
        .iter()
        .find(|(index, _, _)| *index == 0)
        .expect("条 0");
    let second = evidence
        .iter()
        .find(|(index, _, _)| *index == 1)
        .expect("条 1");
    assert!(
        second.1 > first.1,
        "第 2 条的词法证据要强于第 1 条：{evidence:?}"
    );
    let probed = engine.last_probed_segmentations();
    let zero = probed.iter().position(|index| *index == 0);
    let one = probed.iter().position(|index| *index == 1);
    assert!(one.is_some(), "证据更强的第 2 条要拿到名额：{probed:?}");
    assert!(
        zero.is_none_or(|zero| one.expect("checked above") < zero),
        "同结构组里证据更强的第 2 条要排在 parser 靠前的第 1 条之前：{probed:?}"
    );
    assert_eq!(
        query.segmentations[0].to_string(),
        "an xia nan",
        "第 2 条靠高频的 `安下` 胜出"
    );
}
