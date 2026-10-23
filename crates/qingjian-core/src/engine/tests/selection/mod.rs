//! 分段选词的暂存、撤销、确认与候选页数边界。

use std::collections::HashMap;

use qingjian_dictionary::Dictionary;

mod finalization;

use super::CountingLearner;
use crate::candidate::{Candidate, CandidateKind};
use crate::engine::Engine;

fn engine() -> Engine {
    let dictionary = Dictionary::parse("开发\tkai fa\t9000\n开罚\tkai fa\t5000\n开法\tkai fa\t1000\n这里\tzhe li\t9000\n这\tzhe\t900\n里\tli\t800\n吗\tma\t3000\n").unwrap();
    Engine::new(dictionary).with_learner(Box::new(CountingLearner(HashMap::new())))
}

fn candidate(engine: &Engine, text: &str) -> Candidate {
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == text)
        .unwrap_or_else(|| panic!("missing {text}"))
}

#[test]
fn partial_choice_remains_marked_and_backspace_restores_all_original_keys() {
    let mut engine = engine();
    engine.set_input("kai'fa'zhe'li'ma");
    let word = candidate(&engine, "开法");
    assert_eq!(engine.select_candidate(&word), None);
    assert!(engine.query().unwrap().marked_text().starts_with("开法"));
    assert!(engine.raw_preedit().text.starts_with("开法"));
    assert_eq!(engine.learner().choice_weight("kaifa", "开法"), 0);
    assert!(engine.backspace());
    assert_eq!(engine.composition().typed_text(), "kai'fa'zhe'li'ma");
    assert!(engine.query().unwrap().selected.is_empty());
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .any(|c| c.text == "开法")
    );
    assert_eq!(engine.learner().choice_weight("kaifa", "开法"), 0);
}

#[test]
fn consecutive_choices_undo_one_at_a_time_without_losing_the_tail() {
    let mut engine = engine();
    engine.set_input("kaifazhelima");
    let first = candidate(&engine, "开法");
    assert_eq!(engine.select_candidate(&first), None);
    let second = candidate(&engine, "这里");
    assert_eq!(engine.select_candidate(&second), None);
    assert_eq!(engine.raw_preedit().text, "开法这里ma");
    assert!(engine.backspace());
    assert_eq!(engine.raw_preedit().text, "开法zhelima");
    assert!(engine.backspace());
    assert_eq!(engine.raw_preedit().text, "kaifazhelima");
}

#[test]
fn undo_keeps_tail_edits_and_shift_case() {
    let mut engine = engine();
    engine.set_shift_letter_compose(true);
    for c in "Kaifazhelima".chars() {
        engine.push(c);
    }
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    engine.push('n');
    engine.push('e');
    engine.backspace();
    assert_eq!(engine.composition().typed_text(), "Kaifazhelimane");
}

#[test]
fn final_choice_commits_the_whole_phrase_and_learns_each_piece_once() {
    let mut engine = engine();
    engine.set_input("kaifazhelima");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    let second = candidate(&engine, "这里");
    engine.select_candidate(&second);
    let last = candidate(&engine, "吗");
    assert_eq!(
        engine.select_candidate(&last),
        Some("开法这里吗".to_owned())
    );
    assert!(!engine.is_composing());
    assert_eq!(engine.learner().choice_weight("kaifa", "开法"), 1);
    assert_eq!(engine.learner().choice_weight("zheli", "这里"), 1);
    assert_eq!(engine.learner().choice_weight("ma", "吗"), 1);
    assert_eq!(
        engine.learner().choice_weight("kaifazhelima", "开法这里吗"),
        0
    );
}

#[test]
fn cancel_and_replacement_do_not_learn_but_enter_keeps_selected_words() {
    let mut engine = engine();
    for replace in [false, true] {
        engine.set_input("kaifazhelima");
        let first = candidate(&engine, "开法");
        engine.select_candidate(&first);
        if replace {
            engine.set_input("ma");
        } else {
            engine.clear();
        }
        assert_eq!(engine.learner().choice_weight("kaifa", "开法"), 0);
        assert!(!engine.raw_preedit().text.starts_with("开法"));
    }
    engine.set_input("kaifazhelima");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    assert_eq!(engine.take_raw(), "开法zhelima");
    assert!(!engine.is_composing());
    assert_eq!(engine.learner().choice_weight("kaifa", "开法"), 1);
}

#[test]
fn deleting_the_entire_tail_does_not_abandon_the_selected_word() {
    let mut engine = engine();
    engine.set_input("kaifazhelima");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    engine.delete_to_start();
    assert!(engine.is_composing());
    assert_eq!(engine.raw_preedit().text, "开法");
    assert!(engine.backspace());
    assert_eq!(engine.raw_preedit().text, "kaifa");
}

#[test]
fn later_pages_keep_all_single_characters_and_drop_phrases() {
    let mut tsv = String::new();
    for (index, c) in "发罚法乏阀伐筏砝珐垡".chars().enumerate() {
        tsv.push_str(&format!("开{c}\tkai fa\t{}\n", 9000 - index));
    }
    for c in "开凯慨楷铠锴".chars() {
        tsv.push_str(&format!("{c}\tkai\t300\n"));
    }
    let mut engine = Engine::new(Dictionary::parse(&tsv).unwrap());
    engine.set_input("kaifa");
    let original = engine.query().unwrap().candidates.items;
    let singles = original
        .iter()
        .filter(|c| c.kind == CandidateKind::Chinese && c.text.chars().count() == 1)
        .count();
    assert!(singles >= 6);
    for page_size in [1, 3, 9] {
        engine.set_word_candidate_limit(Some(page_size * 2));
        let limited = engine.query().unwrap().candidates.items;
        assert_eq!(limited[0].text, original[0].text);
        assert!(
            limited
                .iter()
                .skip(page_size * 2)
                .all(|c| c.text.chars().count() == 1)
        );
        assert_eq!(
            limited
                .iter()
                .filter(|c| c.text.chars().count() == 1)
                .count(),
            singles
        );
    }
}

#[test]
fn staged_selection_preserves_existing_whole_phrase_learning() {
    let mut engine = engine();
    engine.set_input("kaifazheli");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    let last = candidate(&engine, "这里");
    assert_eq!(engine.select_candidate(&last), Some("开法这里".to_owned()));
    assert_eq!(engine.learner().choice_weight("kaifazheli", "开法这里"), 1);
}
