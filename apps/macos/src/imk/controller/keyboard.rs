//! 英文键盘的可打印字符：直接上屏，全角字符与非 ASCII 标点按系统键盘布局重新解码。

use objc2_app_kit::NSEvent;
use qingjian_platform::Modifiers;

/// 命令键与 Option 组合由应用和系统处理，保留快捷键与组合重音的原生行为。
pub(super) fn english_keyboard_text(event: &NSEvent, pressed: Modifiers) -> Option<String> {
    if pressed.command || pressed.control || pressed.option {
        return None;
    }
    let text = event.characters()?.to_string();
    if printable_ascii(&text) {
        return Some(text);
    }
    // 空字符可能是死键；已组合的重音字母也必须交还系统。全角英数仍按英文键盘重新解码。
    if text.is_empty()
        || text.chars().any(|c| {
            (c.is_alphanumeric() && !('\u{ff01}'..='\u{ff5e}').contains(&c))
                || c.is_control()
                || ('\u{f700}'..='\u{f8ff}').contains(&c)
                || u32::from(c) > 0xffff
        })
    {
        return None;
    }
    event
        .charactersByApplyingModifiers(event.modifierFlags())
        .map(|text| text.to_string())
        .filter(|text| printable_ascii(text))
}

fn printable_ascii(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
}
