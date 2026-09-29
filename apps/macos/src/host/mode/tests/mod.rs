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
    let pending = mode.modifier_event(up(SwitchKey::Shift, 0));
    pending && mode.confirm_pending()
}

mod behavior;
mod control;
mod duration;
mod flow;
