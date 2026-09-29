//! 待定切换、模式设置和直通路由回归。

use super::*;

/// 富文本客户端可能先投递右 Shift 抬起，再投递仍带 Shift 的组合键 KeyDown。
/// 字符完全由客户端提供；这里只按修饰标志作废待定单击。
#[test]
fn late_key_down_cancels_right_shift_chords_in_both_modes() {
    for key in ["A", ".", ",", "'", ";"] {
        for english in [false, true] {
            let mut mode = mode();
            if english {
                assert!(tap_shift(&mut mode));
            }
            mode.modifier_event(down(SwitchKey::Shift, 1, true));
            assert!(mode.modifier_event(up(SwitchKey::Shift, 1)), "{key}");
            assert!(mode.has_pending());
            assert!(!mode.key_down(true, false), "Shift+{key}");
            assert!(!mode.has_pending());
            assert!(!mode.confirm_pending());
            assert_eq!(mode.english(), english, "Shift+{key}");
            assert_eq!(mode.passthrough(), english);
        }
    }
}

#[test]
fn bare_tap_is_confirmed_before_next_unmodified_key() {
    let mut mode = mode();
    mode.modifier_event(down(SwitchKey::Shift, 1, true));
    assert!(mode.modifier_event(up(SwitchKey::Shift, 1)));
    assert!(!mode.english());
    assert!(mode.key_down(false, false));
    assert!(mode.english());
    assert!(!mode.confirm_pending());
}

#[test]
fn settings_and_deactivation_cancel_a_delayed_tap() {
    let mut mode = mode();
    mode.modifier_event(down(SwitchKey::Shift, 1, true));
    assert!(mode.modifier_event(up(SwitchKey::Shift, 1)));
    mode.set_settings(shift(), true);
    assert!(!mode.confirm_pending());
    mode.modifier_event(down(SwitchKey::Shift, 1, true));
    assert!(mode.modifier_event(up(SwitchKey::Shift, 1)));
    mode.cancel_pending();
    assert!(!mode.confirm_pending());
}

/// 没勾 Shift 时按它不切换（`[shortcut] switch_mode = []`）。
#[test]
fn an_unchecked_key_does_not_toggle() {
    let mut mode = ModeState::default();
    mode.set_settings(SwitchKeys::NONE, true);
    assert!(!tap_shift(&mut mode));
    assert!(!mode.english());
}

/// 关掉内置英文模式（`[general] english_mode = false`）：切换键不再切英文，并且退回中文。
#[test]
fn the_master_switch_keeps_it_chinese() {
    let mut mode = mode();
    assert!(mode.toggle());
    assert!(mode.english());
    mode.set_settings(shift(), false);
    assert!(!mode.english());
    assert!(!tap_shift(&mut mode));
    assert!(!mode.english());
}

/// 改配置作废正按着的 Shift：切换键刚换掉，那次按下不该算单击。
#[test]
fn changing_settings_drops_the_pending_tap() {
    let mut mode = mode();
    mode.modifier_event(down(SwitchKey::Shift, 0, true));
    mode.set_settings(shift(), true);
    assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)));
}

/// 英文模式始终纯直通：字母、数字、标点都不该被青简接走。
#[test]
fn english_mode_is_always_pure_passthrough() {
    let mut mode = mode();
    assert!(tap_shift(&mut mode));
    assert!(mode.passthrough());
}

/// 中文模式照常组句：直通判定只认英文模式这一份状态。
#[test]
fn chinese_mode_always_goes_to_the_engine() {
    let mode = mode();
    assert!(!mode.passthrough());
}

/// Caps Lock 只管大小写：它按 `FlagsChanged` 送来，映射不成切换键（`None`），
/// 亮灭多少下都不翻中英模式——模式只有单击切换键这一个来源。
#[test]
fn caps_lock_presses_never_change_the_mode() {
    let mut mode = mode();
    for _ in 0..4 {
        mode.modifier_event(other());
    }
    assert!(!mode.english());
    // 已经在英文模式时也一样：Caps Lock 切不回中文
    assert!(tap_shift(&mut mode));
    for _ in 0..4 {
        mode.modifier_event(other());
    }
    assert!(mode.english());
}
