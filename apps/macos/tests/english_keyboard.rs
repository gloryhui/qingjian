//! 英文键盘事件回归：在 macOS 的 ABC 布局下验证全部标点、命令键与组合重音。
//! AppKit 重新解码会同步到主线程，因此本测试用独立的主线程入口。

use objc2::rc::Retained;
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSPoint, NSString};
use qingjian_platform::Modifiers;

#[path = "../src/imk/controller/keyboard.rs"]
mod keyboard;

use keyboard::english_keyboard_text;

fn main() {
    every_shifted_symbol_uses_the_english_keyboard_layout();
    every_unshifted_symbol_uses_the_english_keyboard_layout();
    full_width_letters_and_digits_use_the_english_keyboard_layout();
    commands_and_option_combinations_stay_with_the_application();
    caps_and_dead_key_text_are_preserved();
    println!("英文键盘原生回归通过：全部 32 个标点、命令键、大小写与组合重音");
}

fn event(key: u16, text: &str, flags: NSEventModifierFlags) -> Retained<NSEvent> {
    let text = NSString::from_str(text);
    NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::ZERO,
        flags,
        0.0,
        0,
        None,
        &text,
        &text,
        false,
        key,
    )
    .expect("创建按键事件")
}

fn decode(key: u16, text: &str, flags: NSEventModifierFlags) -> Option<String> {
    english_keyboard_text(
        &event(key, text, flags),
        Modifiers {
            shift: flags.contains(NSEventModifierFlags::Shift),
            option: flags.contains(NSEventModifierFlags::Option),
            command: flags.contains(NSEventModifierFlags::Command),
            control: flags.contains(NSEventModifierFlags::Control),
        },
    )
}

fn every_shifted_symbol_uses_the_english_keyboard_layout() {
    // 故意给事件塞中文标点：断言实际调用系统布局重新解码，而不是照抄事件里的字符。
    for (key, received, expected) in [
        (18, "！", "!"),
        (19, "＠", "@"),
        (20, "＃", "#"),
        (21, "￥", "$"),
        (23, "％", "%"),
        (22, "……", "^"),
        (26, "＆", "&"),
        (28, "＊", "*"),
        (25, "（", "("),
        (29, "）", ")"),
        (50, "～", "~"),
        (27, "——", "_"),
        (24, "＋", "+"),
        (33, "｛", "{"),
        (30, "｝", "}"),
        (42, "｜", "|"),
        (41, "：", ":"),
        (39, "“", "\""),
        (43, "《", "<"),
        (47, "》", ">"),
        (44, "？", "?"),
    ] {
        assert_eq!(
            decode(key, received, NSEventModifierFlags::Shift).as_deref(),
            Some(expected),
            "key={key}, received={received}"
        );
    }
}

fn every_unshifted_symbol_uses_the_english_keyboard_layout() {
    for (key, received, expected) in [
        (50, "｀", "`"),
        (27, "－", "-"),
        (24, "＝", "="),
        (33, "【", "["),
        (30, "】", "]"),
        (42, "、", "\\"),
        (41, "；", ";"),
        (39, "‘", "'"),
        (43, "，", ","),
        (47, "。", "."),
        (44, "／", "/"),
    ] {
        assert_eq!(
            decode(key, received, NSEventModifierFlags::empty()).as_deref(),
            Some(expected),
            "{received}"
        );
    }
}

fn commands_and_option_combinations_stay_with_the_application() {
    for flag in [
        NSEventModifierFlags::Command,
        NSEventModifierFlags::Control,
        NSEventModifierFlags::Option,
    ] {
        assert_eq!(decode(18, "!", flag | NSEventModifierFlags::Shift), None);
        assert_eq!(decode(0, "a", flag), None);
    }
    for (key, text) in [
        (36, "\r"),
        (48, "\t"),
        (51, "\u{7f}"),
        (53, "\u{1b}"),
        (123, "\u{f702}"),
    ] {
        assert_eq!(
            decode(key, text, NSEventModifierFlags::empty()),
            None,
            "{key}"
        );
    }
}

fn full_width_letters_and_digits_use_the_english_keyboard_layout() {
    for (key, text, flags, expected) in [
        (0, "ａ", NSEventModifierFlags::empty(), "a"),
        (0, "Ａ", NSEventModifierFlags::Shift, "A"),
        (0, "Ａ", NSEventModifierFlags::CapsLock, "A"),
        (18, "１", NSEventModifierFlags::empty(), "1"),
        (49, "　", NSEventModifierFlags::empty(), " "),
    ] {
        assert_eq!(
            decode(key, text, flags).as_deref(),
            Some(expected),
            "{text}"
        );
    }
}

fn caps_and_dead_key_text_are_preserved() {
    assert_eq!(
        decode(0, "A", NSEventModifierFlags::CapsLock).as_deref(),
        Some("A")
    );
    assert_eq!(
        decode(
            0,
            "a",
            NSEventModifierFlags::CapsLock | NSEventModifierFlags::Shift
        )
        .as_deref(),
        Some("a")
    );
    assert_eq!(
        decode(49, " ", NSEventModifierFlags::empty()).as_deref(),
        Some(" ")
    );
    assert_eq!(
        decode(7, "'x", NSEventModifierFlags::empty()).as_deref(),
        Some("'x")
    );
    assert_eq!(decode(14, "é", NSEventModifierFlags::empty()), None);
    assert_eq!(decode(39, "", NSEventModifierFlags::empty()), None);
}
