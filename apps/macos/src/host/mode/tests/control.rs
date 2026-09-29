//! 配置为切换键时的 Control 时长与 Shift 组合键回归。

use std::time::{Duration, Instant};

use qingjian_platform::{SwitchKey, SwitchKeys};

use super::{down, mode, shift, up};

fn at(start: Instant, millis: u64) -> Instant {
    start + Duration::from_millis(millis)
}

#[test]
fn shift_and_configured_control_chord_cannot_switch() {
    let mut mode = mode();
    mode.set_settings(shift().with(SwitchKey::Control, true), true);
    let start = Instant::now();
    mode.modifier_event_at(down(SwitchKey::Shift, 0, true), start);
    mode.modifier_event_at(down(SwitchKey::Control, 0, false), at(start, 50));
    assert!(mode.pressed_at.get().is_none());
    assert!(!mode.modifier_event_at(up(SwitchKey::Shift, 0), at(start, 100)));
    assert!(!mode.modifier_event_at(up(SwitchKey::Control, 0), at(start, 120)));
    assert!(!mode.has_pending());
    assert!(!mode.english());
}

#[test]
fn configured_control_uses_the_same_macos_tap_limit() {
    for (duration_ms, expected) in [(100, true), (1_000, false)] {
        let mut mode = mode();
        mode.set_settings(SwitchKeys::NONE.with(SwitchKey::Control, true), true);
        let start = Instant::now();
        mode.modifier_event_at(down(SwitchKey::Control, 1, true), start);
        assert_eq!(
            mode.modifier_event_at(up(SwitchKey::Control, 1), at(start, duration_ms)),
            expected
        );
        assert_eq!(mode.confirm_pending(), expected);
    }
}
