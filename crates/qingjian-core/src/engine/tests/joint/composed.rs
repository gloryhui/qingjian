//! 未登录组合候选：词库没收录、但由现有字词拼得出来、且有词库共现证据的组合。
//!
//! 语料分三组，正例与反例都用**产品词库**（`assets/lexicon/dict.tsv`）跑，不往词库里加任何测试词：
//!
//! - 正例：`野狼`、`虎猫`、`井水`、`沙漠化` —— 词库里没有这些整词，但两个部分的交界字在某个词条里
//!   同词出现过（`狼子野心`、`照猫画虎`、`水井`、`荒漠化`），必须作为组合候选出现；
//! - 反例：`云豹`、`纸杯`、`竹桥`、`纸伞`、`石阶` —— 同样由两个词库词拼得出来，
//!   但词库里没有任何词条同时含这两个字，不得出现在候选里（这是「不倒笛卡尔积」的证据）；
//! - 同音堆砌：`也狼`、`野浪`、`夜狼`… 同样没有共现证据，不得因为拼得出来就进候选。

use super::*;

/// 组合候选在候选表里的位置上限：必须在第一页（或明确的前 N 个）之内。
const FRONT: usize = 6;

fn texts(engine: &mut Engine, input: &str) -> Vec<String> {
    engine.set_input(input);
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .map(|candidate| candidate.text)
        .collect()
}

fn pick(engine: &mut Engine, input: &str, text: &str) -> Candidate {
    engine.set_input(input);
    let candidate = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == text && candidate.kind == CandidateKind::Chinese)
        .unwrap_or_else(|| panic!("{input} 下找不到 {text}"));
    engine.commit(&candidate);
    candidate
}

#[test]
fn composed_candidate_is_exposed_for_an_unlisted_compound() {
    let mut engine = Engine::new(real_dictionary());
    let all = texts(&mut engine, "ye'lang");
    let position = all.iter().position(|text| text == "野狼");
    assert!(
        position.is_some_and(|position| position < FRONT),
        "野狼 必须在前 {FRONT} 条候选里：{:?}",
        &all[..all.len().min(12)]
    );
    // 完整词仍然是正常候选，而且排在组合候选前面
    let complete = all.iter().position(|text| text == "夜郎").unwrap();
    assert!(complete < position.unwrap(), "{:?}", &all[..8]);
}

#[test]
fn control_cases_prove_the_mechanism_is_generic() {
    let mut engine = Engine::new(real_dictionary());
    for (input, expected) in [
        ("hu'mao", "虎猫"),
        ("jing'shui", "井水"),
        ("sha'mo'hua", "沙漠化"),
    ] {
        let all = texts(&mut engine, input);
        assert!(
            all.iter().any(|text| text == expected),
            "{input} 应当给出组合候选 {expected}：{:?}",
            &all[..all.len().min(12)]
        );
    }
}

#[test]
fn compounds_without_dictionary_evidence_are_not_dumped_into_the_list() {
    let mut engine = Engine::new(real_dictionary());
    for (input, absent) in [
        ("yun'bao", "云豹"),
        ("zhi'bei", "纸杯"),
        ("zhu'qiao", "竹桥"),
        ("zhi'san", "纸伞"),
        ("shi'jie", "石阶"),
    ] {
        let all = texts(&mut engine, input);
        assert!(
            !all.iter().any(|text| text == absent),
            "{input} 不该给出没有共现证据的组合 {absent}"
        );
    }
}

#[test]
fn homophone_pileups_are_not_exposed_as_composed_candidates() {
    let mut engine = Engine::new(real_dictionary());
    let all = texts(&mut engine, "ye'lang");
    for pileup in ["也狼", "野浪", "夜狼", "叶郎", "野郎", "也浪"] {
        assert!(!all.iter().any(|text| text == pileup), "{pileup} 不该出现");
    }
    // 只有 野狼 有共现证据，所以一次只出一条
    let stats = engine.last_joint_stats();
    assert_eq!(stats.generated_candidates, 1, "{stats:?}");
}

/// 同样的拼音形状，只因为词库里有没有共现证据而给出不同结果：门槛是词库推出来的，不是拼音形状。
#[test]
fn the_gate_is_dictionary_evidence_not_pinyin_shape() {
    let with_evidence = Dictionary::parse(
        "野\tye\t9000\n狼\tlang\t8000\n夜郎\tye lang\t9000\n狼子野心\tlang zi ye xin\t100\n",
    )
    .unwrap();
    let without = Dictionary::parse("野\tye\t9000\n狼\tlang\t8000\n夜郎\tye lang\t9000\n").unwrap();
    let mut engine = Engine::new(with_evidence);
    assert!(
        texts(&mut engine, "ye'lang")
            .iter()
            .any(|text| text == "野狼")
    );
    assert!(engine.composes('野', '狼'));
    let mut engine = Engine::new(without);
    assert!(
        !texts(&mut engine, "ye'lang")
            .iter()
            .any(|text| text == "野狼")
    );
    assert!(!engine.composes('野', '狼'));
}

#[test]
fn generated_candidates_are_capped_and_deduplicated() {
    let mut engine = Engine::new(real_dictionary());
    let all = texts(&mut engine, "ye'lang");
    let mut seen = std::collections::HashSet::new();
    for text in &all {
        assert!(seen.insert(text.clone()), "{text} 重复出现");
    }
    assert!(engine.last_joint_stats().generated_candidates <= 3);
}

#[test]
fn composed_candidates_do_not_displace_the_normal_candidates() {
    let mut engine = Engine::new(real_dictionary());
    let all = texts(&mut engine, "ye'lang");
    // 词库直接命中的整词候选一个都不能少
    for expected in ["夜郎", "夜郎自大", "野", "也", "耶"] {
        assert!(all.iter().any(|text| text == expected), "少了 {expected}");
    }
    assert!(all.len() > 20, "候选数不该被组合候选挤掉");
}

/// 组合候选是 `CandidateKind::Chinese`：上屏时按音节消耗输入，并记选择次数。
///
/// 用 `Sentence` 会丢掉选择证据：`commit::sentence_words` 用 parser 首选切分重算路径，
/// 组合候选来自另一条切分时重算不出来，整条学习链就断了。
#[test]
fn first_selection_is_accepted_and_learned() {
    let mut engine = Engine::new(real_dictionary()).with_learner(Box::new(WordLearner::default()));
    let candidate = pick(&mut engine, "ye'lang", "野狼");
    assert_eq!(candidate.kind, CandidateKind::Chinese);
    assert_eq!(candidate.syllables, ["ye", "lang"]);
    assert!(engine.composition().is_empty(), "整段输入被组合候选吃完");
    assert_eq!(engine.learner().choice_weight("yelang", "野狼"), 1);
    // 造用户词仍归自动造词规则管，一次选择还不到
    assert_eq!(engine.learner().weight("野狼"), 0);
}

/// 两个都有共现证据的组合候选，外加一个覆盖整段输入的完整词（组合候选不该越过它）。
/// 词频按产品词库的量级给：完整词与「拆成两个字」的路径分差距要和真实情况相当。
fn competing_dictionary() -> Dictionary {
    Dictionary::parse(
        "净水\tjing shui\t3000\n井\tjing\t9000\n水\tshui\t8000\n警\tjing\t7000\n税\tshui\t6000\n水井\tshui jing\t500\n税警\tshui jing\t400\n",
    )
    .unwrap()
}

/// 用户明确选过之后，组合候选靠选择证据排到同批组合候选的最前面。
#[test]
fn repeated_selection_moves_the_candidate_up() {
    let mut engine =
        Engine::new(competing_dictionary()).with_learner(Box::new(WordLearner::default()));
    let order = |engine: &mut Engine| texts(engine, "jing'shui");
    let before = order(&mut engine);
    assert_eq!(before[0], "净水", "完整词仍在最前：{before:?}");
    let well = before.iter().position(|text| text == "井水").unwrap();
    let tax = before.iter().position(|text| text == "警税").unwrap();
    assert!(well < tax, "默认按路径分排：{before:?}");

    for _ in 0..3 {
        let _ = pick(&mut engine, "jing'shui", "警税");
    }
    assert_eq!(engine.learner().choice_weight("jingshui", "警税"), 3);
    let after = order(&mut engine);
    let well = after.iter().position(|text| text == "井水").unwrap();
    let tax = after.iter().position(|text| text == "警税").unwrap();
    assert!(tax < well, "选过之后应当靠前：{after:?}");
    assert_eq!(after[0], "净水", "完整词仍然在最前：{after:?}");
}

/// 组合候选与整句胜出候选同文本时不重复插入。
#[test]
fn a_composition_equal_to_the_winning_sentence_is_not_duplicated() {
    let mut engine =
        Engine::new(competing_dictionary()).with_learner(Box::new(WordLearner::default()));
    let all = texts(&mut engine, "jing'shui");
    for text in ["净水", "井水", "警税"] {
        assert_eq!(
            all.iter().filter(|candidate| *candidate == text).count(),
            1,
            "{text} 出现了多次：{all:?}"
        );
    }
}

/// 自动造词仍然走原有规则：同一段拼音里连着选出两个词、记够次数就造用户词，
/// 之后整段拼音直接命中完整词，不再依赖组合候选。
#[test]
fn auto_word_rule_still_turns_the_composition_into_a_user_word() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = Engine::new(real_dictionary()).with_learner(Box::new(learner));
    for round in 1..=2 {
        let _ = pick(&mut engine, "ye'lang", "野");
        engine.set_input("lang");
        let _ = pick(&mut engine, "lang", "狼");
        assert_eq!(shared.lock().unwrap().0.len(), usize::from(round == 2));
    }
    assert_eq!(shared.lock().unwrap().0, ["野狼"]);
    let all = texts(&mut engine, "ye'lang");
    assert_eq!(all[0], "野狼", "用户词生效后整段拼音直接命中完整词");
}

/// 模型自己造出来、用户没选过的组合不会被学成用户词。
#[test]
fn unselected_compositions_are_not_learned() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = Engine::new(real_dictionary()).with_learner(Box::new(learner));
    for _ in 0..5 {
        let _ = texts(&mut engine, "ye'lang");
        let _ = texts(&mut engine, "hu'mao");
    }
    assert!(shared.lock().unwrap().0.is_empty(), "没选过就不该造词");
    assert_eq!(engine.learner().choice_weight("yelang", "野狼"), 0);
}

/// 神经分不是个人证据：模型给组合候选打高分，不产生词频、选择次数或用户词。
///
/// 有两条以上组合候选时它们才进 scorer（一条时无需重排），所以这里用一对竞争词库。
#[test]
fn neural_score_is_not_personal_evidence() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(competing_dictionary())
        .with_learner(Box::new(learner))
        .with_sentence_scorer(
            Box::new(PrefersWholeSentence {
                target: "警税",
                seen: Arc::clone(&seen),
            }),
            Some(1.0),
            None,
            None,
        );
    let all = texts(&mut engine, "jing'shui");
    assert_eq!(all[0], "净水", "完整词仍由词级排序占首位：{all:?}");
    assert_eq!(all[1], "警税", "神经分把组合候选排到井水前面：{all:?}");
    assert!(seen.lock().unwrap().iter().any(|text| text == "警税"));
    assert!(seen.lock().unwrap().iter().any(|text| text == "井水"));
    assert_eq!(engine.learner().choice_weight("jingshui", "警税"), 0);
    assert_eq!(engine.learner().weight("警税"), 0);
    assert!(shared.lock().unwrap().0.is_empty(), "神经分不该造用户词");
}

/// 没有共现证据时，用户自己的接续证据也能让组合候选站住。
#[test]
fn personal_evidence_admits_a_composition_without_dictionary_support() {
    // 井+水 没有共现词条，默认不出组合候选；完整词 净水 保住首选
    let dictionary =
        || Dictionary::parse("净水\tjing shui\t900000\n井\tjing\t9000\n水\tshui\t8000\n").unwrap();
    let mut plain = Engine::new(dictionary());
    let before = texts(&mut plain, "jing'shui");
    assert_eq!(before[0], "净水");
    assert!(!before.iter().any(|text| text == "井水"));
    assert!(!plain.composes('井', '水'));

    let mut learner = WordLearner::default();
    for _ in 0..8 {
        learner.record_transition(sentence::Context::START, "井", 2);
        learner.record_transition(sentence::Context::after("井"), "水", 2);
    }
    let mut engine = Engine::new(dictionary()).with_learner(Box::new(learner));
    let with = texts(&mut engine, "jing'shui");
    assert!(
        with.iter().any(|text| text == "井水"),
        "个人证据应当让没有共现证据的组合也能出现：{with:?}"
    );
}

/// 完整覆盖、无占位：末尾音节还没打完时不造组合。
#[test]
fn incomplete_trailing_syllable_yields_no_composition() {
    let mut engine = Engine::new(real_dictionary());
    let all = texts(&mut engine, "ye'lan");
    assert!(!all.iter().any(|text| text == "野狼"));
    assert_eq!(engine.last_joint_stats().generated_candidates, 0);
}
