//! Shift 大写在行内预编辑、候选窗拼音行与原样上屏之间保持一致。

use crate::dictionary::Dictionary;
use crate::{Engine, ShuangpinScheme};
use qingjian_dictionary::WordList;

fn engine() -> Engine {
    let mut engine = Engine::new(Dictionary::default());
    engine.set_shift_letter_compose(true);
    engine
}

fn assert_preedit(engine: &Engine, expected: &str, cursor: usize) {
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), expected);
    assert_eq!(query.marked_cursor(), cursor);
    let window_text: String = query
        .marked_segments()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(window_text, expected);
    assert_eq!(query.segments_cursor(), cursor);
}

#[test]
fn shifted_uuid_preview_keeps_case_after_every_key() {
    for input in ["UUID", "Uuid", "uUID"] {
        let mut engine = engine();
        for (index, c) in input.char_indices() {
            engine.push(c);
            assert_preedit(&engine, &input[..index + 1], index + 1);
        }
        assert_eq!(engine.take_raw(), input);
    }
}

#[test]
fn shifted_english_only_fallback_keeps_case() {
    let words = WordList::parse("import\timport\t5000\n").unwrap();
    let mut engine = engine().with_english(words);
    engine.set_input("IMPORT");
    assert_preedit(&engine, "IMPORT", 6);
    assert_eq!(engine.take_raw(), "IMPORT");
}

#[test]
fn shifted_expression_preview_keeps_case() {
    let mut engine = engine();
    engine.set_input("V1+2");
    assert_preedit(&engine, "V1+2", 4);
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "3");
    assert_eq!(engine.take_raw(), "V1+2");
}

#[test]
fn shifted_raw_preview_matches_the_candidate_and_raw_commit() {
    let mut engine = engine();
    engine.set_input("Ab-9");
    assert_preedit(&engine, "Ab-9", 4);
    let candidate = engine.query().unwrap().candidates.items[0].clone();
    assert_eq!(candidate.text, "Ab-9");
    assert_eq!(engine.commit(&candidate), "Ab-9");
    engine.set_input("Ab-9");
    assert_eq!(engine.take_raw(), "Ab-9");
}

#[test]
fn shifted_case_survives_pinyin_boundaries_and_cursor_edits() {
    let mut engine = engine();
    for input in ["KaiFa", "Kai'Fa", "Kai''Fa"] {
        engine.set_input(input);
        assert_preedit(&engine, "Kai'Fa", 6);
        engine.move_cursor_left();
        engine.move_cursor_left();
        assert_preedit(&engine, "Kai'Fa", if input.contains('\'') { 4 } else { 3 });
        engine.move_cursor_home();
        assert_preedit(&engine, "Kai'Fa", 0);
        engine.move_cursor_end();
        assert_eq!(engine.take_raw(), input);
    }
}

#[test]
fn shifted_shuangpin_remainder_still_displays_decoded_syllables() {
    let mut engine = engine();
    engine.set_shuangpin(Some(ShuangpinScheme::Xiaohe));
    engine.set_input("NiHc");
    assert_preedit(&engine, "ni'hao", 6);
    engine.move_cursor_left();
    engine.move_cursor_left();
    assert_preedit(&engine, "ni'hao", 2);
    assert_eq!(engine.take_raw(), "NiHc");
}
