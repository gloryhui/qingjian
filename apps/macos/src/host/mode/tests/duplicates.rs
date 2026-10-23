//! 按 Codex 日志重放重复的 Shift 按下 / 抬起，覆盖待定切换、组合键与长按。

use std::time::{Duration, Instant};

use qingjian_platform::{SwitchKey, SwitchKeys};

use super::{down, mode, other, up};

#[test]
fn duplicated_press_and_release_toggle_once_in_both_modes() {
    for key in [SwitchKey::Shift, SwitchKey::Control] {
        for slot in [0, 1] {
            for english in [false, true] {
                let mut mode = mode();
                mode.set_settings(SwitchKeys::NONE.with(key, true), true);
                if english {
                    mode.toggle();
                }
                let start = Instant::now();
                let (shift, control) = (key == SwitchKey::Shift, key == SwitchKey::Control);
                assert!(!mode.modifier_flags_at(down(key, slot, true), shift, control, start));
                assert!(!mode.modifier_flags_at(
                    down(key, slot, true),
                    shift,
                    control,
                    start + Duration::from_millis(4),
                ));
                assert!(mode.switch_pressed());
                assert!(!mode.has_pending());
                assert!(mode.modifier_flags_at(
                    up(key, slot),
                    false,
                    false,
                    start + Duration::from_millis(92),
                ));
                assert!(!mode.modifier_flags_at(
                    up(key, slot),
                    false,
                    false,
                    start + Duration::from_millis(94),
                ));
                assert!(mode.has_pending());
                assert!(mode.confirm_pending());
                assert_eq!(mode.english(), !english);
                assert!(!mode.confirm_pending());
            }
        }
    }
}

#[test]
fn duplicated_press_does_not_restart_the_long_hold_clock() {
    let mut mode = mode();
    let start = Instant::now();
    mode.modifier_flags_at(down(SwitchKey::Shift, 0, true), true, false, start);
    mode.modifier_flags_at(
        down(SwitchKey::Shift, 0, true),
        true,
        false,
        start + Duration::from_millis(490),
    );
    assert!(!mode.modifier_flags_at(
        up(SwitchKey::Shift, 0),
        false,
        false,
        start + Duration::from_millis(600),
    ));
    assert!(!mode.confirm_pending());
    assert!(!mode.english());
}

#[test]
fn duplicate_releases_preserve_pending_until_the_next_key() {
    for chord in [false, true] {
        let mut mode = mode();
        mode.modifier_flags(down(SwitchKey::Shift, 1, true), true, false);
        assert!(mode.modifier_flags(up(SwitchKey::Shift, 1), false, false));
        assert!(!mode.modifier_flags(up(SwitchKey::Shift, 1), false, false));
        assert!(mode.has_pending());
        assert_eq!(mode.key_down(chord, false), !chord);
        assert_eq!(mode.english(), !chord);
        assert!(!mode.confirm_pending());
    }
}

#[test]
fn duplicate_events_do_not_restore_an_interrupted_chord() {
    let mut mode = mode();
    mode.modifier_flags(down(SwitchKey::Shift, 0, true), true, false);
    mode.key_down(true, false);
    mode.modifier_flags(down(SwitchKey::Shift, 0, true), true, false);
    assert!(!mode.modifier_flags(up(SwitchKey::Shift, 0), false, false));
    assert!(!mode.modifier_flags(up(SwitchKey::Shift, 0), false, false));
    assert!(!mode.confirm_pending());
    assert!(!mode.english());
}

#[test]
fn overlapping_shifts_never_toggle_and_the_next_tap_recovers() {
    for first in [0, 1] {
        let mut mode = mode();
        mode.modifier_flags(down(SwitchKey::Shift, first, true), true, false);
        mode.modifier_flags(down(SwitchKey::Shift, 1 - first, true), true, false);
        assert!(!mode.modifier_flags(up(SwitchKey::Shift, first), true, false));
        assert!(!mode.modifier_flags(up(SwitchKey::Shift, 1 - first), false, false));
        assert!(!mode.confirm_pending());
        mode.modifier_flags(down(SwitchKey::Shift, first, true), true, false);
        assert!(mode.modifier_flags(up(SwitchKey::Shift, first), false, false));
        assert!(mode.confirm_pending());
        assert!(mode.english());
    }
}

#[test]
fn other_modifiers_still_cancel_pending_and_repeated_chords() {
    let mut mode = mode();
    mode.modifier_flags(down(SwitchKey::Shift, 0, true), true, false);
    mode.modifier_flags(down(SwitchKey::Shift, 0, false), true, true);
    assert!(!mode.modifier_flags(up(SwitchKey::Shift, 0), false, false));
    assert!(!mode.confirm_pending());
    mode.modifier_flags(down(SwitchKey::Shift, 0, true), true, false);
    assert!(mode.modifier_flags(up(SwitchKey::Shift, 0), false, false));
    mode.modifier_flags(other(), false, false);
    assert!(!mode.confirm_pending());
    assert!(!mode.english());
}

/// Shift+B 的字母事件到达前，重复的 Shift 按下不能提前启动切换定时器。
#[test]
fn duplicated_shift_press_never_switches_before_the_book_keydown() {
    for slot in [0, 1] {
        for english in [false, true] {
            let mut mode = mode();
            if english {
                mode.toggle();
            }
            let start = Instant::now();
            assert!(!mode.modifier_flags_at(
                down(SwitchKey::Shift, slot, true),
                true,
                false,
                start
            ));
            assert!(!mode.modifier_flags_at(
                down(SwitchKey::Shift, slot, true),
                true,
                false,
                start + Duration::from_millis(80),
            ));
            assert!(!mode.confirm_pending(), "Shift is still held before B");
            assert!(!mode.key_down(true, false), "Shift+B must not switch modes");
            assert!(!mode.modifier_flags_at(
                up(SwitchKey::Shift, slot),
                false,
                false,
                start + Duration::from_millis(120),
            ));
            assert!(!mode.confirm_pending());
            assert_eq!(mode.english(), english);
        }
    }
}

/// 轻按只需几十毫秒；重复投递既不能吞掉松开，也不能要求下一次长按才能恢复。
#[test]
fn repeated_quick_taps_switch_in_both_directions_without_a_long_hold() {
    let mut mode = mode();
    let start = Instant::now();
    for index in 0..10 {
        let at = start + Duration::from_millis(index * 200);
        mode.modifier_flags_at(down(SwitchKey::Shift, 0, true), true, false, at);
        mode.modifier_flags_at(
            down(SwitchKey::Shift, 0, true),
            true,
            false,
            at + Duration::from_millis(2),
        );
        assert!(mode.modifier_flags_at(
            up(SwitchKey::Shift, 0),
            false,
            false,
            at + Duration::from_millis(60)
        ));
        assert!(!mode.modifier_flags_at(
            up(SwitchKey::Shift, 0),
            false,
            false,
            at + Duration::from_millis(62)
        ));
        assert!(mode.confirm_pending());
        assert_eq!(mode.english(), index % 2 == 0);
    }
}
