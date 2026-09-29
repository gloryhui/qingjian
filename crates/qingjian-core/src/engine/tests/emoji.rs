//! emoji。

use super::*;

fn has_emoji(engine: &Engine) -> bool {
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .any(|candidate| candidate.kind == CandidateKind::Emoji)
}

#[test]
fn emoji_candidates_are_off_by_default_and_toggle_for_chinese_and_english() {
    let words = WordList::parse("smile\n").unwrap();
    let table = EmojiTable::parse("开发\t👨‍💻\nsmile\t😀\n").unwrap();
    let mut engine = engine().with_english(words).with_emoji(table);

    engine.set_input("kaifa");
    assert!(!has_emoji(&engine));
    engine.set_emoji_candidates(true);
    assert!(has_emoji(&engine));
    engine.set_emoji_candidates(false);
    assert!(!has_emoji(&engine));

    engine.set_english_mode(true);
    engine.set_input("smile");
    assert!(!has_emoji(&engine));
    engine.set_emoji_candidates(true);
    assert!(has_emoji(&engine));
    engine.set_emoji_candidates(false);
    assert!(!has_emoji(&engine));
}

#[test]
fn english_words_bring_their_emoji_and_the_emoji_consumes_the_whole_input() {
    let words = WordList::parse("smile\nsmiled\n").unwrap();
    let table = EmojiTable::parse("smile\t😀 😄\n笑\t😄\n").unwrap();
    let mut engine = engine().with_english(words).with_emoji(table);
    engine.set_emoji_candidates(true);
    engine.set_input("smile");
    let items = engine.query().unwrap().candidates.items;
    let position = items
        .iter()
        .position(|c| c.kind == CandidateKind::English && c.text == "smile")
        .unwrap();
    let emoji = items[position + 1].clone();
    assert_eq!(emoji.kind, CandidateKind::Emoji);
    assert_eq!(emoji.text, "😀");
    assert_eq!(emoji.reading.as_deref(), Some("smile"));
    assert_eq!(engine.commit(&emoji), "😀");
    assert!(engine.composition().is_empty());
}

#[test]
fn emoji_follow_their_word_and_consume_its_syllables() {
    let table = EmojiTable::parse("开发\t👨‍💻 🛠️ 🧑‍💻\n开\t🔓\n").unwrap();
    let mut engine = engine().with_emoji(table);
    engine.set_emoji_candidates(true);
    engine.set_input("kaifazhe");
    let query = engine.query().unwrap();
    let items = &query.candidates.items;
    let kaifa = items.iter().position(|c| c.text == "开发").unwrap();
    assert_eq!(items[kaifa + 1].text, "👨‍💻");
    assert_eq!(items[kaifa + 1].kind, CandidateKind::Emoji);
    assert_eq!(items[kaifa + 1].reading.as_deref(), Some("开发"));
    assert_eq!(items[kaifa + 2].text, "🛠️");
    // 每个词最多两个，一次最多三个
    assert_eq!(
        items
            .iter()
            .filter(|c| c.kind == CandidateKind::Emoji)
            .count(),
        3
    );
    let emoji = items[kaifa + 1].clone();
    assert_eq!(engine.commit(&emoji), "👨‍💻");
    assert_eq!(engine.composition().text(), "zhe");
}
