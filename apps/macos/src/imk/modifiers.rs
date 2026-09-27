//! 修饰键与切换键的识别。IMK 的 `inputText:client:` 不带事件对象，Caps Lock 只能从系统当前状态读。

use objc2_app_kit::{NSEvent, NSEventModifierFlags};
use qingjian_platform::SwitchKey;

/// Caps Lock 亮着：只管大小写，这一次按键整个交给应用（大写由系统键盘布局给），不改中英模式。
pub fn caps_lock_on() -> bool {
    NSEvent::modifierFlags_class().contains(NSEventModifierFlags::CapsLock)
}

/// 修饰键的 `keyCode`（ANSI 物理键）对应哪个单击切换键，左右各算一个。
///
/// Caps Lock（58）不在表里：它已经不是中英切换键。`Ctrl + Alt + Space` 是组合键，macOS 不接
/// （`⌃⌥Space` 是 VoiceOver 在读内容，截走它会影响辅助功能）。
pub fn switch_key_of(key_code: u16) -> Option<SwitchKey> {
    match key_code {
        56 | 60 => Some(SwitchKey::Shift),
        59 | 62 => Some(SwitchKey::Control),
        _ => None,
    }
}

/// 一条 `FlagsChanged` 里这个切换键是按下还是抬起：事件带的修饰标志是变化**之后**的状态。
pub fn switch_key_down(event: &NSEvent, key: SwitchKey) -> bool {
    let flag = match key {
        SwitchKey::Shift => NSEventModifierFlags::Shift,
        SwitchKey::Control => NSEventModifierFlags::Control,
        SwitchKey::CtrlAltSpace => return false,
    };
    event.modifierFlags().contains(flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shift 单击是切换键；Caps Lock、Tab、字母都不算。
    #[test]
    fn only_the_two_shifts_and_two_controls_are_switch_keys() {
        for key_code in [56, 60] {
            assert_eq!(switch_key_of(key_code), Some(SwitchKey::Shift));
        }
        for key_code in [59, 62] {
            assert_eq!(switch_key_of(key_code), Some(SwitchKey::Control));
        }
        // 58 = Caps Lock，48 = Tab，36 = 回车，0 = A
        for key_code in [58, 48, 36, 0, 51] {
            assert_eq!(switch_key_of(key_code), None, "{key_code}");
        }
    }
}
