//! 未登录组合候选：词库没收录、但由现有字词拼得出来的组合。
//!
//! **准入不看「两个部分的交界字有没有在别的词条里同词出现过」**——那个信号回答不了
//! 「这次输入是不是就要这个组合」，`云豹`／`纸杯`／`纸伞`／`石阶`／`藤壶`／`竹桥` 都是正常汉语组合，
//! 不能因为没有共现词条就不给候选。共现只做**排序加成**（`COMPOSED_SUPPORT_BONUS`），
//! 所以 `ye'lang` 下 `野狼` 排在 `也浪` 前面，但两者都在池子里。
//!
//! 准入是结构 + 分数：只按**敲的原样读音**建格（模糊音 / 敲错变体拼出来的不算新词）、
//! 两个部分都是词库词、完整覆盖、无占位、配对分数落在最优路径 `COMPOSED_MARGIN` 以内；
//! 最优路径整段已经是一个**常用完整词**时整批不出（`COMPOSED_DOMINANT_WORD_FREQUENCY`）。
//!
//! 语料分几组，正例与反例都用**产品词库**（`assets/lexicon/dict.tsv`）跑，不往词库里加任何测试词：
//!
//! - 正例：`野狼`、`虎猫`、`井水`、`沙漠化` —— 词库里没有这些整词，由两个词库词拼出来，必须可选；
//! - 「没有共现证据」的正例：`云豹`、`纸杯`、`纸伞`、`石阶`、`藤壶` —— 在受控词库（零共现词条）上
//!   照样进池子、照样可选，证明准入与共现无关；
//! - 真正的垃圾反例：结构不合格（末尾音节没打完 / 靠模糊音凑出来）或差最优路径太远的组合，
//!   一条都不进候选；同音堆砌（`也浪`、`也郎`…）会出现但排在 `野狼` 之后且总数 ≤ 3 条。

use super::*;
use crate::sentence::LanguageModel;

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

/// 「交界字有没有在别的词条里同框」不再是准入条件。
///
/// 受控词库里每条组合的两个字都没有任何共现词条，只要整段拼音没有更好的读法，
/// 它们照样进池子、照样能被选中——准入只看结构、完整覆盖与 margin。
#[test]
fn compounds_are_admitted_without_dictionary_cooccurrence() {
    for (input, wanted, parts) in [
        ("ye'lang", "野狼", [("野", "ye"), ("狼", "lang")]),
        ("teng'hu", "藤壶", [("藤", "teng"), ("壶", "hu")]),
        ("yun'bao", "云豹", [("云", "yun"), ("豹", "bao")]),
        ("zhi'bei", "纸杯", [("纸", "zhi"), ("杯", "bei")]),
        ("zhi'san", "纸伞", [("纸", "zhi"), ("伞", "san")]),
        ("shi'jie", "石阶", [("石", "shi"), ("阶", "jie")]),
    ] {
        // 每个音节给两个字：目标组合的两个字，外加两组同样合法的竞争字
        let mut source = String::new();
        for (character, syllable) in parts {
            let filler = if syllable == "ye" { "叶" } else { "河" };
            source.push_str(&format!(
                "{character}\t{syllable}\t9000\n{filler}\t{syllable}\t8000\n"
            ));
        }
        let mut engine = Engine::new(Dictionary::parse(&source).unwrap());
        engine.set_input(input);
        let all = texts(&mut engine, input);
        assert!(
            all.iter().any(|text| text == wanted),
            "{input} 的 {wanted} 应当可选（没有任何共现证据）：{all:?}"
        );
        let left = parts[0].0.chars().next().unwrap();
        let right = parts[1].0.chars().next().unwrap();
        assert!(!engine.composes(left, right), "{wanted} 本就没有共现证据");
    }
}

/// 真实词库里这几个正常汉语组合既不在词库里、也没有任何共现证据——
/// 它们出不出现只由**结构条件 + margin + 名额**决定，与共现无关。
///
/// 这几条要真正排进前几名需要语义证据（产品语料 LM 或神经分，见
/// `a_language_model_that_knows_the_compound_lifts_it_to_the_front`），
/// PR 与 `docs/notes/issue-13-joint-oov.md` 标注了 `WAITING_FOR_PRODUCT_LM_EVAL`。
#[test]
fn real_dictionary_compounds_have_no_cooccurrence_evidence_at_all() {
    let engine = Engine::new(real_dictionary());
    for (left, right) in [
        ('云', '豹'),
        ('纸', '杯'),
        ('纸', '伞'),
        ('石', '阶'),
        ('藤', '壶'),
        ('野', '狼'),
    ] {
        let supported = engine.composes(left, right);
        if (left, right) == ('野', '狼') {
            assert!(supported, "野狼 有 狼子野心 作证，靠共现加成排到组合池第一");
        } else {
            assert!(
                !supported,
                "{left}{right} 在词库里没有任何共现词条，出不出候选与共现无关"
            );
        }
    }
}

/// 词库共现只影响排序：`ye'lang` 下 `野狼` 因此排在 `也浪` 前面，但两者都在池子里。
#[test]
fn dictionary_cooccurrence_only_ranks_never_admits() {
    let mut engine = Engine::new(real_dictionary());
    engine.set_input("ye'lang");
    let _ = engine.query().unwrap();
    let pool = engine.last_composed_pool();
    let wolf = pool
        .iter()
        .position(|text| text == "野狼")
        .expect("野狼在池子里");
    let other = pool
        .iter()
        .position(|text| text == "也浪")
        .expect("也浪也在池子里（准入不看共现）");
    assert!(wolf < other, "有共现证据的排前面：{pool:?}");
    assert!(engine.composes('野', '狼'));
    assert!(!engine.composes('也', '浪'));
}

/// 真正的垃圾反例：结构上就不合格的组合一条都不进池子。
#[test]
fn structurally_invalid_compositions_never_enter_the_pool() {
    // 末尾音节没打完：整段边界本身就不确定
    let mut engine = Engine::new(real_dictionary());
    let _ = texts(&mut engine, "ye'l");
    assert!(
        engine.last_composed_pool().is_empty(),
        "{:?}",
        engine.last_composed_pool()
    );

    // 两个部分必须是词库里的词；敲错 / 模糊音命中的边（代价 > 0）不算
    let dictionary =
        Dictionary::parse("野\tye\t9000\n狼\tlang\t8000\n夜郎\tye lang\t500\n拦\tlan\t9000\n")
            .unwrap();
    let mut engine = Engine::new(dictionary);
    let _ = texts(&mut engine, "ye'lang");
    let pool = engine.last_composed_pool();
    assert!(pool.iter().any(|text| text == "野狼"), "{pool:?}");
    assert!(
        !pool.iter().any(|text| text == "野拦"),
        "靠模糊音 / 敲错边凑出来的组合不进池子：{pool:?}"
    );
}

/// 与前一条最优路径差太远的组合不进池子（margin 是唯一与分数有关的准入条件）。
#[test]
fn compositions_far_behind_the_best_path_stay_out() {
    let dictionary =
        Dictionary::parse("蛋糕\tdan gao\t900000\n当\tdang\t100\n奥\tao\t100\n").unwrap();
    let mut engine = Engine::new(dictionary);
    let all = texts(&mut engine, "dangao");
    assert_eq!(all[0], "蛋糕");
    assert!(
        !all.iter().any(|text| text == "当奥"),
        "落后最优路径 10 nat 以上的组合不该占名额：{all:?}"
    );
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
        "净水\tjing shui\t800\n井\tjing\t9000\n水\tshui\t8000\n警\tjing\t7000\n税\tshui\t6000\n水井\tshui jing\t500\n税警\tshui jing\t400\n",
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

/// 完整覆盖、无占位：末尾音节还没打完时一条组合都不造。
#[test]
fn incomplete_trailing_syllable_yields_no_composition() {
    let mut engine = Engine::new(real_dictionary());
    let all = texts(&mut engine, "ye'l");
    assert!(!all.iter().any(|text| text == "野狼"));
    assert_eq!(engine.last_joint_stats().generated_candidates, 0);
}

/// 附加词库热加载：词库索引随 `set_extra_dictionaries` 立刻失效重建，不用重启进程。
#[test]
fn composition_index_follows_extra_dictionaries_at_runtime() {
    let base = || {
        Dictionary::parse(
            "野\tye\t9000\n狼\tlang\t8000\n夜郎\tye lang\t500\n叶\tye\t9500\n郎\tlang\t8500\n",
        )
        .unwrap()
    };
    let mut engine = Engine::new(base());
    engine.set_input("ye'lang");
    let _ = engine.query().unwrap();
    assert!(!engine.composes('野', '狼'), "主词库里没有共现证据");
    let before = engine.last_composed_pool();
    let wolf = before
        .iter()
        .position(|text| text == "野狼")
        .expect("野狼在池子里");
    assert!(wolf > 0, "没有共现证据时排不到组合池第一位：{before:?}");

    // 导入一张含「狼子野心」的附加词库：新证据必须立刻生效
    engine.set_extra_dictionaries(vec![
        Dictionary::parse("狼子野心\tlang zi ye xin\t100\n").unwrap(),
    ]);
    assert!(engine.composes('野', '狼'), "导入后立刻生效，不用重启");
    engine.set_input("ye'lang");
    let _ = engine.query().unwrap();
    assert_eq!(
        engine.last_composed_pool()[0],
        "野狼",
        "新证据立刻影响排序：{:?}",
        engine.last_composed_pool()
    );

    // 移除：旧证据立刻消失
    engine.set_extra_dictionaries(Vec::new());
    assert!(!engine.composes('野', '狼'), "移除后旧证据立刻消失");
    engine.set_input("ye'lang");
    let _ = engine.query().unwrap();
    assert_ne!(
        engine.last_composed_pool()[0],
        "野狼",
        "排序回到没有证据时的样子"
    );
}

/// 语言模型认识这个组合时它就该被选出来。
///
/// 本环境没有仓库外的产品工件 `data/generated/lm.qj`（PR 标注 `WAITING_FOR_PRODUCT_LM_EVAL`），
/// 这里用受控模型证明链路：只要模型给出 `藤`→`壶` 的接续证据，`藤壶` 就从池子后段升到第一条。
#[test]
fn a_language_model_that_knows_the_compound_lifts_it_to_the_front() {
    struct KnowsTheCompound;

    impl LanguageModel for KnowsTheCompound {
        fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
            match (previous, word) {
                (Some("藤"), "壶") => Some(-0.5),
                _ => None,
            }
        }
    }

    let plain = {
        let mut engine = Engine::new(real_dictionary());
        let _ = texts(&mut engine, "teng'hu");
        engine.last_composed_pool()
    };
    assert_ne!(plain[0], "藤壶", "没有模型时排在更常用的同音组合后面");

    let mut engine = Engine::new(real_dictionary()).with_language_model(Box::new(KnowsTheCompound));
    let all = texts(&mut engine, "teng'hu");
    assert_eq!(engine.last_composed_pool()[0], "藤壶");
    assert!(
        all.iter().any(|text| text == "藤壶"),
        "模型认识就该选得到：{all:?}"
    );
}
