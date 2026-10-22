//! P0 回归（Issue #16）：子词的个人选择次数不能把拼字整句顶到完整词前面；
//! 用户明确选过整个结果时，仍然可以把它推回第一位。
//!
//! 根因在 [`Engine::should_prefer_complete_word`] 的旁路条件：`Conversion::personal_bonus`
//! 是路径上**逐词**累加的，`的`、`一个` 各自被选过就会 > 0。旧逻辑拿这个值放行，
//! 于是 `di'yi'ge` 的 `的一个`、`de'yuan'sheng` 的 `的原生` 会越过 `第一个` / `德元升`。
//! 放行证据应当是「本输入串下明确选过这个完整结果」（整句候选上屏、整段分几次选完）。

use super::*;
use crate::sentence::LanguageModel;
use std::collections::HashMap;

/// 产品词库的缩样：两个完整词 + 组成它们的高频碎片。
/// 碎片的词频与个人选择次数正是实机上把整句顶上去的那份证据。
const DICT: &str = "\
德元升\tde yuan sheng\t3000
第一个\tdi yi ge\t19032
的\tde\t10189450
的\tdi\t21162
得\tde\t291636
一个\tyi ge\t300000
原生\tyuan sheng\t2000
原声\tyuan sheng\t2000
";

fn counted(pairs: &[(&str, u32)]) -> CountingLearner {
    CountingLearner(
        pairs
            .iter()
            .map(|(text, count)| ((*text).to_owned(), *count))
            .collect::<HashMap<_, _>>(),
    )
}

/// 只认「的 + 一个」这一条高频接续：让拼字整句在纯词频下也足以成为整句首选，
/// 复现产品上「完整词存在、整句却明显更像人话」的竞争场景。
struct SplitBigram;

impl LanguageModel for SplitBigram {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "的") => Some(-0.5),
            (Some("的"), "一个") => Some(-0.5),
            _ => Some(-5.0),
        }
    }
}

fn engine_with(learner: CountingLearner) -> Engine {
    Engine::new(Dictionary::parse(DICT).unwrap())
        .with_learner(Box::new(learner))
        .with_language_model(Box::new(SplitBigram))
}

fn top(engine: &mut Engine, input: &str) -> String {
    engine.set_input(input);
    engine.query().unwrap().candidates.items[0].text.clone()
}

#[test]
fn clean_profile_prefers_the_complete_word() {
    let mut engine = engine_with(counted(&[]));
    assert_eq!(top(&mut engine, "deyuansheng"), "德元升");
    assert_eq!(top(&mut engine, "diyige"), "第一个");
}

#[test]
fn child_word_frequency_does_not_lift_a_split_sentence_over_a_complete_word() {
    // 用户常选「的」「一个」「得」「原生」这些碎片，但从没选过整个结果。
    let mut engine = engine_with(counted(&[
        ("的", 500),
        ("一个", 300),
        ("得", 500),
        ("原生", 200),
    ]));
    assert_eq!(top(&mut engine, "deyuansheng"), "德元升");
    assert_eq!(top(&mut engine, "diyige"), "第一个");
    // 显式单引号与连续全拼是同一个选择键，行为必须一致。
    assert_eq!(top(&mut engine, "de'yuan'sheng"), "德元升");
    assert_eq!(top(&mut engine, "di'yi'ge"), "第一个");
}

#[test]
fn explicit_choice_of_the_whole_result_still_lifts_it() {
    // 用户在本输入串下明确选过整句「的一个」：这是允许越过完整词的证据。
    let mut engine = engine_with(counted(&[
        ("的", 500),
        ("一个", 300),
        ("diyige\t的一个", 2),
    ]));
    assert_eq!(top(&mut engine, "diyige"), "的一个");
    assert_eq!(top(&mut engine, "di'yi'ge"), "的一个");
    // 选择按输入串作用域生效：同一次学习不改变别的输入。
    assert_eq!(top(&mut engine, "deyuansheng"), "德元升");
}

#[test]
fn choosing_the_pieces_one_by_one_teaches_the_whole_result() {
    // 端到端：分两次选「的」+「一个」，`finish_buffer` 记下整段选择；
    // 旧逻辑只看子词 personal_bonus 时这里会一直是「第一个」，新逻辑让明确选择生效。
    let mut engine = engine_with(counted(&[("的", 500), ("一个", 300)]));
    assert_eq!(top(&mut engine, "diyige"), "第一个");

    engine.set_input("diyige");
    let de = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == "的" && candidate.kind == CandidateKind::Chinese)
        .expect("候选里应有 的");
    engine.commit(&de);
    assert_eq!(engine.composition().text(), "yige");
    let yi = engine.query().unwrap().candidates.items[0].clone();
    assert_eq!(yi.text, "一个");
    engine.commit(&yi);
    assert!(engine.composition().is_empty());

    assert!(engine.learner().choice_weight("diyige", "的一个") > 0);
    assert_eq!(top(&mut engine, "diyige"), "的一个");
}

#[test]
fn committing_a_sentence_records_the_explicit_choice() {
    // 整句候选本身也要留下证据，否则「选过整句」永远无法与其他个人加分区分。
    let dictionary = Dictionary::parse("想\txiang\t9000\n开发\tkai fa\t20000\n").unwrap();
    let mut engine =
        Engine::new(dictionary).with_learner(Box::new(CountingLearner(HashMap::new())));
    engine.set_input("xiangkaifa");
    let sentence = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.kind == CandidateKind::Sentence)
        .expect("应有整句候选 想开发");
    assert_eq!(sentence.text, "想开发");
    engine.commit(&sentence);
    assert!(engine.learner().choice_weight("xiangkaifa", "想开发") > 0);
}

#[test]
fn repeated_queries_on_one_snapshot_are_deterministic() {
    let mut engine = engine_with(counted(&[("的", 500), ("一个", 300)]));
    let first = top(&mut engine, "diyige");
    assert_eq!(first, "第一个");
    for _ in 0..4 {
        assert_eq!(top(&mut engine, "diyige"), first);
    }
}
