//! 中英切换状态机回归：单击、组合键与模式直通。

use qingjian_platform::SwitchKey;

use super::*;

fn shift() -> SwitchKeys {
    SwitchKeys::NONE.with(SwitchKey::Shift, true)
}

/// 配置套上：开着内置英文模式、勾着单击 Shift（与配置文件缺省一致）。
fn mode() -> ModeState {
    let mut mode = ModeState::default();
    mode.set_settings(shift(), true);
    mode
}

/// 一个切换键按下：`slot` 是左 / 右键位，`bare` 为那一刻没按着别的修饰键。
fn down(key: SwitchKey, slot: u8, bare: bool) -> ModifierEvent {
    ModifierEvent {
        switch: Some(key),
        slot,
        bare,
    }
}

/// 一个切换键抬起。
fn up(key: SwitchKey, slot: u8) -> ModifierEvent {
    ModifierEvent {
        switch: Some(key),
        slot,
        bare: false,
    }
}

/// 别的修饰键（Caps Lock、⌘、⌥）：映射不成切换键，壳报 `None`。
fn other() -> ModifierEvent {
    ModifierEvent {
        switch: None,
        slot: 0,
        bare: false,
    }
}

/// 裸按一次左 Shift（按下再抬起），返回这次单击有没有翻模式。
fn tap_shift(mode: &mut ModeState) -> bool {
    mode.modifier_event(down(SwitchKey::Shift, 0, true));
    mode.modifier_event(up(SwitchKey::Shift, 0))
}

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
