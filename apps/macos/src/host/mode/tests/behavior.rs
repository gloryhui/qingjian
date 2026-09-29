//! 中英切换的物理键位、组合键与恢复回归。

use super::*;

/// 单击 Shift：中文 → 英文，再单击：英文 → 中文。
#[test]
fn a_shift_tap_toggles_both_ways() {
    let mut mode = mode();
    assert!(!mode.english());
    assert!(tap_shift(&mut mode));
    assert!(mode.english());
    assert!(tap_shift(&mut mode));
    assert!(!mode.english());
}

/// `Shift + 任何别的键` 都不切换：普通字母、数字、Tab（壳从 KeyDown 报 `interrupt`）、
/// 别的修饰键（这里是按着 Shift 又按 ⌘）。
#[test]
fn shift_with_another_key_never_toggles() {
    for case in ["普通键", "别的修饰键"] {
        let mut mode = mode();
        mode.modifier_event(down(SwitchKey::Shift, 0, true));
        match case {
            "普通键" => {
                mode.interrupt();
            }
            _ => {
                mode.modifier_event(other());
            }
        }
        assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)), "{case}");
        assert!(!mode.english(), "{case}");
    }
}

/// 按住 Shift 连打几个键再抬起：中间每一下都作废单击，一次都不切。
#[test]
fn holding_shift_across_several_keys_does_not_toggle() {
    let mut mode = mode();
    mode.modifier_event(down(SwitchKey::Shift, 0, true));
    for _ in 0..5 {
        mode.interrupt();
    }
    assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)));
    assert!(!mode.english());
}

/// 按下 Shift 之前已经按着 ⌥ / ⌘ / ⌃（那次按下不是裸按）：抬起不切模式。
#[test]
fn shift_tapped_while_a_modifier_is_already_held_never_toggles() {
    for held in ["⌥", "⌘", "⌃"] {
        let mut mode = mode();
        // 先按下的那个修饰键按 `FlagsChanged` 送来，映射不成切换键
        mode.modifier_event(other());
        mode.modifier_event(down(SwitchKey::Shift, 0, false));
        assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)), "{held}");
        assert!(!mode.english(), "{held}");
    }
}

/// 左右 Shift 一起按（左下、右下、左上、右上）：一次都不切——聚合标志分不出左右，靠键位记。
#[test]
fn both_shifts_together_never_toggle() {
    let mut mode = mode();
    for slot in [0u8, 1] {
        mode.modifier_event(down(SwitchKey::Shift, slot, true));
    }
    for slot in [0u8, 1] {
        assert!(!mode.modifier_event(up(SwitchKey::Shift, slot)), "{slot}");
    }
    assert!(!mode.english());
}

/// 左右 Shift 各单击一次（一次抬起之后才按下另一只）：各切一次。
#[test]
fn two_separate_shift_taps_each_toggle() {
    let mut mode = mode();
    for slot in [0u8, 1] {
        mode.modifier_event(down(SwitchKey::Shift, slot, true));
        assert!(mode.modifier_event(up(SwitchKey::Shift, slot)), "{slot}");
        assert!(mode.confirm_pending(), "{slot}");
    }
    assert!(!mode.english());
}

/// 抬起事件漏了（输入法是按住 Shift 才被激活的）之后靠对账恢复：下一次裸按照常切。
#[test]
fn resync_recovers_from_a_missed_release() {
    let mut mode = mode();
    for slot in [0u8, 1] {
        mode.modifier_event(down(SwitchKey::Shift, slot, true));
    }
    assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)));
    mode.resync(false, false);
    assert!(tap_shift(&mut mode));
    assert!(mode.english());
}

/// 正按着 Shift 时系统标志里已经没有它：对账把那次作废，抬起也不再切。
#[test]
fn resync_drops_a_tap_whose_key_the_system_no_longer_reports() {
    let mut mode = mode();
    mode.modifier_event(down(SwitchKey::Shift, 0, true));
    mode.resync(false, false);
    assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)));
    assert!(!mode.english());
}

/// 作废过的那次之后重新裸按仍然有效：状态干净，不留幽灵按下。
#[test]
fn a_fresh_tap_works_after_a_discarded_one() {
    let mut mode = mode();
    mode.modifier_event(down(SwitchKey::Shift, 0, true));
    mode.interrupt();
    assert!(!mode.modifier_event(up(SwitchKey::Shift, 0)));
    assert!(tap_shift(&mut mode));
    assert!(mode.english());
}
