//! 单击切换键的判定：按下切换键到抬起之间没插进别的键，就是一次单击（供各壳翻中 / 英模式用）。
//!
//! 只有状态机在这里，**键码到 [`SwitchKey`] 的映射留在各平台壳**（Windows 是虚拟键码、macOS 是 `keyCode`、
//! Linux 是 Fcitx5 的 keysym），壳把每次按下 / 抬起喂进来：不是切换键就传 `None`。
//! 只在壳自己的线程上用，不进协议也不跨进程，所以不做序列化（`Cell` 不是 `Sync`）。

use std::cell::Cell;

use crate::config::{SwitchKey, SwitchKeys};

/// 正按着哪个切换键；一次按下只 fire 一次。
#[derive(Debug, Default)]
pub struct KeyTap {
    /// 按下了哪个切换键、之后还没有别的键插进来。
    pressed: Cell<Option<SwitchKey>>,
}

impl KeyTap {
    /// 任一键按下：`switch` 是它对应的切换键（普通字母、数字、Tab 这类传 `None`），`repeat` 为自动重复。
    ///
    /// 不是切换键、或勾着的键里没有它，就作废这次按下：之后抬起 Shift 不算单击（`Shift + A` 不能切模式）。
    /// 自动重复不算新的按下。
    pub fn key_down(&self, switch: Option<SwitchKey>, repeat: bool, keys: SwitchKeys) {
        let Some(key) = switch.filter(|key| keys.contains(*key)) else {
            self.cancel();
            return;
        };
        if repeat {
            return;
        }
        // 两个不同的切换键一起按（Windows 的 Ctrl + Shift 是系统换布局的快捷键）不算单击
        self.pressed.set(match self.pressed.get() {
            Some(other) if other != key => None,
            _ => Some(key),
        });
    }

    /// 任一键抬起：勾着的切换键单独抬起且中间没插进别的键，返回 `true`（一次按下只算一次）。
    pub fn key_up(&self, switch: Option<SwitchKey>, keys: SwitchKeys) -> bool {
        let Some(key) = switch.filter(|key| keys.contains(*key)) else {
            return false;
        };
        if self.pressed.get() == Some(key) {
            self.pressed.set(None);
            true
        } else {
            false
        }
    }

    /// 作废正按着的切换键：按下之后发生了别的事（系统热键把第二个键截走、输入源被抢），这次抬起不算单击。
    pub fn cancel(&self) {
        self.pressed.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(key: SwitchKey) -> SwitchKeys {
        SwitchKeys::NONE.with(key, true)
    }

    /// 单击 Shift 算一次，抬起之后不再有第二次；中间插进别的键就不算。
    #[test]
    fn shift_tap_fires_only_when_nothing_else_interrupts() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_down(Some(SwitchKey::Shift), false, keys);
        assert!(tap.key_up(Some(SwitchKey::Shift), keys));
        assert!(!tap.key_up(Some(SwitchKey::Shift), keys));

        tap.key_down(Some(SwitchKey::Shift), false, keys);
        tap.key_down(None, false, keys); // 中间插了一个别的键
        assert!(!tap.key_up(Some(SwitchKey::Shift), keys));
    }

    /// 只有勾上的键算单击：没勾 Shift 时按它抬起不切换，一个都不勾时修饰键单击都不算。
    #[test]
    fn only_checked_keys_fire() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Control);
        tap.key_down(Some(SwitchKey::Shift), false, keys);
        assert!(!tap.key_up(Some(SwitchKey::Shift), keys));
        tap.key_down(Some(SwitchKey::Control), false, keys);
        assert!(tap.key_up(Some(SwitchKey::Control), keys));

        for keys in [SwitchKeys::NONE, only(SwitchKey::CtrlAltSpace)] {
            tap.key_down(Some(SwitchKey::Shift), false, keys);
            assert!(!tap.key_up(Some(SwitchKey::Shift), keys));
            tap.key_down(Some(SwitchKey::Control), false, keys);
            assert!(!tap.key_up(Some(SwitchKey::Control), keys));
        }
    }

    /// 两个组合 `Ctrl + Shift` 一起按：谁抬起都不算单击（系统换布局的键，不是输入法切换）。
    #[test]
    fn both_taps_work_when_both_are_checked_but_not_together() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift).with(SwitchKey::Control, true);
        tap.key_down(Some(SwitchKey::Shift), false, keys);
        assert!(tap.key_up(Some(SwitchKey::Shift), keys));
        tap.key_down(Some(SwitchKey::Control), false, keys);
        assert!(tap.key_up(Some(SwitchKey::Control), keys));

        tap.key_down(Some(SwitchKey::Control), false, keys);
        tap.key_down(Some(SwitchKey::Shift), false, keys);
        assert!(!tap.key_up(Some(SwitchKey::Shift), keys));
        assert!(!tap.key_up(Some(SwitchKey::Control), keys));
    }

    /// 外部打断（系统热键截走了组合里的第二个键）作废正按着的那次。
    #[test]
    fn cancel_drops_the_pending_tap() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Control);
        tap.key_down(Some(SwitchKey::Control), false, keys);
        tap.cancel();
        assert!(!tap.key_up(Some(SwitchKey::Control), keys));
    }

    /// 按住不放产生的自动重复不算新按下。
    #[test]
    fn auto_repeat_does_not_rearm() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_down(Some(SwitchKey::Shift), false, keys);
        assert!(tap.key_up(Some(SwitchKey::Shift), keys));
        tap.key_down(Some(SwitchKey::Shift), true, keys);
        assert!(!tap.key_up(Some(SwitchKey::Shift), keys));
    }
}
