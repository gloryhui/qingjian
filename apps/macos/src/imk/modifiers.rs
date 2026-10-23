//! 修饰键与切换键的识别。IMK 的 `inputText:client:` 不带事件对象，Caps Lock 只能从系统当前状态读。

use objc2_app_kit::{NSEvent, NSEventModifierFlags};
use qingjian_platform::key_tap::ModifierEvent;
use qingjian_platform::{KeyCombo, Modifiers, SwitchKey};

/// 组合键精确匹配修饰键；主回车与数字键盘回车等价，Tab 不参与。
pub fn matches_combo(
    combo: KeyCombo,
    pressed: Modifiers,
    key_code: u16,
    typed: Option<&str>,
) -> bool {
    combo.modifiers == pressed
        && if combo.key == '\r' {
            matches!(key_code, 36 | 76)
        } else {
            typed.is_some_and(|text| text.eq_ignore_ascii_case(&combo.key.to_string()))
        }
}

/// Caps Lock 亮着：只管大小写，这一次按键整个交给应用（大写由系统键盘布局给），不改中英模式。
pub fn caps_lock_on() -> bool {
    NSEvent::modifierFlags_class().contains(NSEventModifierFlags::CapsLock)
}

/// 修饰键的 `keyCode`（ANSI 物理键）对应哪个单击切换键、是它的第几个键位（左右各一只）。
///
/// Caps Lock（58）不在表里：它已经不是中英切换键。`Ctrl + Alt + Space` 是组合键，macOS 不接
/// （`⌃⌥Space` 是 VoiceOver 在读内容，截走它会影响辅助功能）。
pub fn switch_key_of(key_code: u16) -> Option<(SwitchKey, u8)> {
    match key_code {
        56 => Some((SwitchKey::Shift, 0)),
        60 => Some((SwitchKey::Shift, 1)),
        59 => Some((SwitchKey::Control, 0)),
        62 => Some((SwitchKey::Control, 1)),
        _ => None,
    }
}

/// 一条 `FlagsChanged` 解码成 [`ModifierEvent`]：是哪个切换键的哪只键、按下这一刻是不是裸按。
///
/// 键位由 [`qingjian_platform::key_tap::KeyTap`] 记录；控制器把聚合标志一并传给 ModeState，
/// 先忽略重复的按下 / 抬起，再送进单击状态机。聚合标志分不清左右两只，仍需键位记录。
pub fn modifier_event(event: &NSEvent) -> ModifierEvent {
    match switch_key_of(event.keyCode()) {
        Some((key, slot)) => ModifierEvent {
            switch: Some(key),
            slot,
            bare: bare_press(key, event.modifierFlags()),
        },
        // ⌘ / ⌥ / Caps Lock：不是切换键，它作废正按着的单击
        None => ModifierEvent {
            switch: None,
            slot: 0,
            bare: false,
        },
    }
}

/// 这一刻是不是裸按这个切换键：⌘ / ⌥ / fn 与另一个切换键都算「别的修饰键」。
///
/// Caps Lock 不算——它是大写锁定，亮着时聚合标志一直带着，算进去就永远没法裸按了。
fn bare_press(key: SwitchKey, flags: NSEventModifierFlags) -> bool {
    let others = match key {
        SwitchKey::Shift => {
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Control
        }
        SwitchKey::Control => {
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Shift
        }
        SwitchKey::CtrlAltSpace => return false,
    } | NSEventModifierFlags::Function;
    !flags.intersects(others)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_combo_accepts_both_returns_with_exact_modifiers_only() {
        let combo = KeyCombo::COMMIT_TRANSLATION_DEFAULT;
        for key in [36, 76] {
            assert!(matches_combo(combo, Modifiers::OPTION, key, None));
            assert!(!matches_combo(combo, Modifiers::default(), key, None));
            assert!(!matches_combo(
                combo,
                "option+shift".parse().unwrap(),
                key,
                None
            ));
        }
        assert!(!matches_combo(combo, Modifiers::OPTION, 48, Some("\t")));
        let custom: KeyCombo = "control+e".parse().unwrap();
        assert!(matches_combo(
            custom,
            "control".parse().unwrap(),
            14,
            Some("E")
        ));
        assert!(!matches_combo(
            custom,
            "control".parse().unwrap(),
            14,
            Some("EE")
        ));
    }

    /// Shift 与 Control 的左右两只各占一个键位；Caps Lock、Tab、字母都不算。
    #[test]
    fn only_the_two_shifts_and_two_controls_are_switch_keys() {
        assert_eq!(switch_key_of(56), Some((SwitchKey::Shift, 0)));
        assert_eq!(switch_key_of(60), Some((SwitchKey::Shift, 1)));
        assert_eq!(switch_key_of(59), Some((SwitchKey::Control, 0)));
        assert_eq!(switch_key_of(62), Some((SwitchKey::Control, 1)));
        // 58 = Caps Lock，48 = Tab，36 = 回车，0 = A，51 = 退格
        for key_code in [58, 48, 36, 0, 51] {
            assert_eq!(switch_key_of(key_code), None, "{key_code}");
        }
    }

    /// 裸按判定看的是**别的**修饰键：按 Shift 时 ⌘ / ⌥ / ⌃ / fn 亮着都不算裸按，Caps Lock 不算数。
    #[test]
    fn shift_is_bare_only_without_another_modifier() {
        use NSEventModifierFlags as F;
        let caps = F::CapsLock;
        assert!(bare_press(SwitchKey::Shift, F::empty()));
        assert!(bare_press(SwitchKey::Shift, F::Shift));
        assert!(
            bare_press(SwitchKey::Shift, F::Shift | caps),
            "⇪ 只管大小写"
        );
        for held in [F::Command, F::Option, F::Control, F::Function] {
            assert!(!bare_press(SwitchKey::Shift, F::Shift | held), "{held:?}");
        }
    }

    /// 按 Control 时另一个切换键（⇧）也算「别的修饰键」：`Ctrl + Shift` 是系统换布局的键。
    #[test]
    fn control_is_not_bare_while_shift_is_held() {
        use NSEventModifierFlags as F;
        assert!(bare_press(SwitchKey::Control, F::Control));
        assert!(!bare_press(SwitchKey::Control, F::Control | F::Shift));
        assert!(!bare_press(SwitchKey::Control, F::Control | F::Command));
        // 反向也一样：按着 ⌃ 再按 ⇧ 不是裸按
        assert!(!bare_press(SwitchKey::Shift, F::Shift | F::Control));
    }
}
