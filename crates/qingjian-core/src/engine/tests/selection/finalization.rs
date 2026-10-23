//! 分段暂存对原样确认、译词、补全与双拼的兼容边界。

use qingjian_dictionary::{Dictionary, WordList};

use super::{candidate, engine};
use crate::candidate::{Language, PartOfSpeech, Sense, Translation};
use crate::engine::{Engine, EngineSession};
use crate::shuangpin::Scheme;

#[test]
fn absent_translation_keeps_pending_selection_without_learning() {
    let mut engine = engine();
    engine.set_input("kaifazhelima");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    let tail = candidate(&engine, "这里");
    assert_eq!(engine.commit_translation(&tail, 0), None);
    assert_eq!(engine.raw_preedit().text, "开法zhelima");
    assert_eq!(engine.learner().choice_weight("kaifa", "开法"), 0);
    assert!(engine.backspace());
}

#[test]
fn translation_confirmation_includes_selected_prefix_once() {
    let mut engine = engine();
    engine.set_input("kaifazheli");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    let mut tail = candidate(&engine, "这里");
    tail.translation = Some(Translation::new(
        Language::English,
        vec![Sense {
            part_of_speech: Some(PartOfSpeech::Adverb),
            text: "here".to_owned(),
            reading: None,
            fresh: false,
        }],
    ));
    assert_eq!(
        engine.commit_translation(&tail, 0),
        Some("开法here".to_owned())
    );
    assert!(!engine.is_composing());
}

#[test]
fn cloud_sentence_confirmation_keeps_the_selected_prefix() {
    let mut engine = engine();
    engine.set_input("kaifazhelima");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    assert_eq!(engine.accept_prediction("这里吗"), "开法这里吗");
    assert!(!engine.is_composing());
}

#[test]
fn question_tail_does_not_clear_the_selected_prefix() {
    let mut engine = engine();
    engine.set_input("kaifazheli");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    engine.delete_to_start();
    engine.push('?');
    assert!(!engine.bare_question());
    assert_eq!(engine.restore_bare_question(false), None);
    assert!(engine.raw_preedit().text.starts_with("开法"));
}

#[test]
fn shuangpin_undo_restores_keys_and_raw_preedit_cursor() {
    let dict = Dictionary::parse("开发\tkai fa\t9000\n你\tni\t2000\n").unwrap();
    let mut engine = Engine::new(dict);
    engine.set_shuangpin(Some(Scheme::Xiaohe));
    engine.set_shuangpin_raw_preedit(true);
    engine.set_input("kdfani");
    let first = candidate(&engine, "开发");
    assert_eq!(engine.select_candidate(&first), None);
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "开发ni");
    assert_eq!(query.marked_cursor(), 4);
    assert!(engine.backspace());
    assert_eq!(engine.composition().typed_text(), "kdfani");
}

#[test]
fn single_character_pages_do_not_truncate_english_mode_completions() {
    let words = WordList::parse("book\tbook\t5000\nbooks\tbooks\t4000\nbooking\tbooking\t3000\nbookstore\tbookstore\t2000\n").unwrap();
    let mut engine = engine().with_english(words);
    engine.set_word_candidate_limit(Some(2));
    engine.set_english_mode(true);
    engine.set_input("book");
    assert!(engine.query().unwrap().candidates.items.len() > 2);
}

#[test]
fn pending_selection_is_isolated_and_restored_with_its_input_session() {
    let mut engine = engine();
    engine.set_input("kaifazheli");
    let first = candidate(&engine, "开法");
    engine.select_candidate(&first);
    let mut session = EngineSession::default();
    engine.swap_session(&mut session);
    assert!(!engine.is_composing());
    engine.set_input("ma");
    engine.swap_session(&mut session);
    assert_eq!(engine.raw_preedit().text, "开法zheli");
    assert!(engine.backspace());
    assert_eq!(engine.raw_preedit().text, "kaifazheli");
    engine.swap_session(&mut session);
    assert_eq!(engine.raw_preedit().text, "ma");
}
