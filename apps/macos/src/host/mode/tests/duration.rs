//! 用注入的单调时间验证短按、长按与乱序事件，不依赖真实等待。

use std::time::{Duration, Instant};

use qingjian_platform::SwitchKey;

use super::{down, mode, other, shift, tap_shift, up};

fn at(start: Instant, millis: u64) -> Instant {
    start + Duration::from_millis(millis)
}

fn timed_shift(mode: &mut super::ModeState, slot: u8, duration_ms: u64) -> bool {
    let start = Instant::now();
    assert!(!mode.modifier_event_at(down(SwitchKey::Shift, slot, true), start));
    mode.modifier_event_at(up(SwitchKey::Shift, slot), at(start, duration_ms))
}

#[test]
fn short_shift_tap_toggles_both_shifts_in_both_modes() {
    for slot in [0, 1] {
        for english in [false, true] {
            let mut mode = mode();
            if english {
                assert!(tap_shift(&mut mode));
            }
            assert!(timed_shift(&mut mode, slot, 100), "slot={slot}");
            assert!(mode.has_pending());
            assert!(mode.confirm_pending());
            assert_eq!(mode.english(), !english, "slot={slot}");
            assert!(mode.pressed_at.get().is_none());
        }
    }
}

#[test]
fn long_shift_hold_does_not_toggle_in_either_mode() {
    for slot in [0, 1] {
        for english in [false, true] {
            let mut mode = mode();
            if english {
                assert!(tap_shift(&mut mode));
            }
            assert!(!timed_shift(&mut mode, slot, 1_000), "slot={slot}");
            assert!(!mode.has_pending());
            assert!(!mode.confirm_pending());
            assert_eq!(mode.english(), english, "slot={slot}");
            assert_eq!(mode.passthrough(), english);
            assert!(mode.pressed_at.get().is_none());
        }
    }
}

#[test]
fn threshold_boundary_includes_exactly_500_milliseconds() {
    for (millis, expected) in [(499, true), (500, true), (501, false)] {
        let mut mode = mode();
        assert_eq!(timed_shift(&mut mode, 0, millis), expected, "{millis}ms");
        assert_eq!(mode.has_pending(), expected, "{millis}ms");
        assert_eq!(mode.confirm_pending(), expected, "{millis}ms");
    }
}

#[test]
fn short_shift_chords_still_cancel_in_both_modes() {
    for key in ["A", ".", ",", "'", ";", "Tab", "1", "/"] {
        for english in [false, true] {
            let mut mode = mode();
            if english {
                assert!(tap_shift(&mut mode));
            }
            let start = Instant::now();
            mode.modifier_event_at(down(SwitchKey::Shift, 1, true), start);
            mode.interrupt();
            assert!(mode.pressed_at.get().is_none(), "Shift+{key}");
            assert!(!mode.modifier_event_at(up(SwitchKey::Shift, 1), at(start, 100)));
            assert!(!mode.has_pending());
            assert_eq!(mode.english(), english, "Shift+{key}");
        }
    }
}

#[test]
fn another_modifier_and_both_shifts_clear_the_pressed_time() {
    for interruption in ["Option", "Command", "Function", "Control", "other Shift"] {
        let mut mode = mode();
        let start = Instant::now();
        mode.modifier_event_at(down(SwitchKey::Shift, 0, true), start);
        assert!(mode.pressed_at.get().is_some());
        match interruption {
            "Option" | "Command" | "Function" => {
                mode.modifier_event_at(other(), at(start, 50));
            }
            "Control" => {
                mode.modifier_event_at(down(SwitchKey::Control, 0, false), at(start, 50));
            }
            _ => {
                mode.modifier_event_at(down(SwitchKey::Shift, 1, true), at(start, 50));
            }
        }
        assert!(mode.pressed_at.get().is_none(), "{interruption}");
        assert!(!mode.modifier_event_at(up(SwitchKey::Shift, 0), at(start, 100)));
        if interruption == "other Shift" {
            assert!(!mode.modifier_event_at(up(SwitchKey::Shift, 1), at(start, 120)));
        }
        assert!(!mode.has_pending(), "{interruption}");
    }
}

#[test]
fn delayed_shift_keydown_cancels_pending_in_both_modes() {
    for english in [false, true] {
        let mut mode = mode();
        if english {
            assert!(tap_shift(&mut mode));
        }
        assert!(timed_shift(&mut mode, 1, 100));
        assert!(mode.has_pending());
        assert!(!mode.key_down(true, false));
        assert!(!mode.has_pending());
        assert!(!mode.confirm_pending());
        assert_eq!(mode.english(), english);
    }
}

#[test]
fn another_modifier_during_grace_cancels_the_pending_tap() {
    let mut mode = mode();
    assert!(timed_shift(&mut mode, 0, 100));
    assert!(mode.has_pending());
    mode.modifier_event_at(other(), Instant::now());
    assert!(!mode.has_pending());
    assert!(!mode.confirm_pending());
    assert!(!mode.english());
    assert!(mode.pressed_at.get().is_none());
}

#[test]
fn short_tap_is_confirmed_before_unmodified_key_but_long_hold_is_not() {
    for (duration_ms, expected) in [(100, true), (1_000, false)] {
        let mut mode = mode();
        assert_eq!(timed_shift(&mut mode, 0, duration_ms), expected);
        assert_eq!(mode.key_down(false, false), expected);
        assert!(!mode.has_pending());
        assert_eq!(mode.english(), expected);
        assert!(!mode.confirm_pending());
    }
}

#[test]
fn settings_deactivation_and_resync_clear_pressed_time_and_pending() {
    for cleanup in ["settings", "deactivation", "resync"] {
        let mut mode = mode();
        let start = Instant::now();
        mode.modifier_event_at(down(SwitchKey::Shift, 0, true), start);
        match cleanup {
            "settings" => mode.set_settings(shift(), true),
            "deactivation" => mode.cancel_pending(),
            _ => mode.resync(false, false),
        }
        assert!(mode.pressed_at.get().is_none(), "{cleanup}");
        assert!(!mode.modifier_event_at(up(SwitchKey::Shift, 0), at(start, 100)));
        mode.resync(false, false);
        assert!(!mode.has_pending(), "{cleanup}");

        assert!(timed_shift(&mut mode, 1, 100));
        match cleanup {
            "settings" => mode.set_settings(shift(), true),
            "deactivation" => mode.cancel_pending(),
            _ => mode.cancel_pending(),
        }
        assert!(mode.pressed_at.get().is_none(), "{cleanup}");
        assert!(!mode.has_pending(), "{cleanup}");
        assert!(!mode.confirm_pending(), "{cleanup}");
    }
}

#[test]
fn discarded_hold_cannot_poison_the_next_tap() {
    let mut mode = mode();
    assert!(!timed_shift(&mut mode, 0, 1_000));
    assert!(timed_shift(&mut mode, 0, 100));
    assert!(mode.confirm_pending());
    assert!(mode.english());
    assert!(timed_shift(&mut mode, 1, 100));
    assert!(mode.confirm_pending());
    assert!(!mode.english());
}
