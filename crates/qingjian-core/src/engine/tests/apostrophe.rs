//! 显式拼音分隔符在候选上屏和缓冲区编辑时的边界。

use super::*;

fn apostrophe_engine() -> Engine {
    let dictionary =
        Dictionary::parse("开\tkai\t20000\n发\tfa\t10000\n开发\tkai fa\t9000\n").unwrap();
    Engine::new(dictionary)
}

#[test]
fn selecting_a_prefix_word_consumes_its_following_separators() {
    for input in ["kai'fa", "kai''fa", "'kai'''fa"] {
        let mut engine = apostrophe_engine();
        engine.set_input(input);
        let first = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == "开")
            .unwrap();
        assert_eq!(engine.commit(&first), "开", "{input}");
        assert_eq!(engine.composition().text(), "fa", "{input}");
        let next = engine.query().unwrap().candidates.items[0].clone();
        assert_eq!(engine.commit(&next), "发", "{input}");
        assert!(engine.composition().is_empty(), "{input}");
    }
}

#[test]
fn selecting_a_full_word_across_repeated_separators_clears_the_buffer() {
    for input in ["kai'fa", "kai''fa", "'kai'''fa'"] {
        let mut engine = apostrophe_engine();
        engine.set_input(input);
        let word = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == "开发")
            .unwrap();
        assert_eq!(engine.commit(&word), "开发", "{input}");
        assert!(engine.composition().is_empty(), "{input}");
    }
}

#[test]
fn raw_commit_backspace_and_clear_preserve_separator_boundaries() {
    let mut engine = apostrophe_engine();
    engine.set_input("kai''fa");
    assert_eq!(engine.take_raw(), "kai''fa");
    assert!(engine.composition().is_empty());

    engine.set_input("kai''fa");
    assert!(engine.backspace());
    assert!(engine.backspace());
    assert_eq!(engine.composition().text(), "kai''");
    assert!(engine.backspace());
    assert_eq!(engine.composition().text(), "kai'");
    engine.clear();
    assert!(engine.composition().is_empty());
}
