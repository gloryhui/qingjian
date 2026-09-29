//! 单击切换键的判定：裸按一个物理键位、按下到抬起之间没插进别的键，才是一次单击（供各壳翻中 / 英模式用）。
//!
//! 只有状态机在这里，**键码到 [`SwitchKey`] 与键位号的映射留在各平台壳**（Windows 是虚拟键码、macOS 是
//! `keyCode`、Linux 是 Fcitx5 的 keysym），壳把每次修饰键事件解码成 [`ModifierEvent`] 喂进来。
//! 只在壳自己的线程上用，不进协议也不跨进程，所以不做序列化（`Cell` 不是 `Sync`）。

use std::cell::Cell;

use crate::config::{SwitchKey, SwitchKeys};

/// 一个物理修饰键的一次按下 / 抬起，由壳从平台事件解码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierEvent {
    /// 这个物理键对应哪个切换键。普通键与 ⌘ / ⌥ 这类别的修饰键传 `None`：它作废正按着的单击。
    pub switch: Option<SwitchKey>,

    /// 同一切换键下的物理键位（左 / 右各一个，缺省 0 / 1）。`switch` 为 `None` 时忽略。
    ///
    /// 键位号是必需的：左右两只 Shift 在**聚合**的修饰标志里是同一个位，只认标志会把
    /// 「左 Shift 按着又按右 Shift」当成一次单击。
    pub slot: u8,

    /// 按下这一刻是不是**裸按**：没同时按着 ⌘ / ⌥ / fn 与另一个切换键。Caps Lock 不算（它只管大小写）。
    ///
    /// 缺了这个，`⌥ 按住 → 单击 Shift` 会被当成一次裸按而切模式——那是组合快捷键，不是切换。
    pub bare: bool,
}

/// 正按着的物理键位，加上「按下之后没被别的键打断」的那次单击。
#[derive(Debug, Default)]
pub struct KeyTap {
    /// 每个切换键此刻按着哪些键位（bit 号 = [`ModifierEvent::slot`]）。
    held: [Cell<u8>; SLOTS],

    /// 正按着、按下到抬起之间没插进别的键的切换键；一次按下只 fire 一次。
    armed: Cell<Option<SwitchKey>>,
}

/// [`KeyTap::held`] 的长度：够放三个切换键的键位记录。
const SLOTS: usize = 3;

/// 一个键位号最多记到 8（bit 掩码是 `u8`），超出的按 0 处理。
const SLOT_BITS: u8 = u8::BITS as u8 - 1;

fn slots_of(key: SwitchKey) -> usize {
    match key {
        SwitchKey::Shift => 0,
        SwitchKey::Control => 1,
        // 组合键不作为物理键位送来（各壳自己认），这里只是把数组占满
        SwitchKey::CtrlAltSpace => 2,
    }
}

impl KeyTap {
    /// 当前是否还有一次未被组合键打断的按下；供平台壳自行判断单击时长。
    pub fn is_armed(&self) -> bool {
        self.armed.get().is_some()
    }

    /// 一次物理修饰键事件：命中一次单击返回 `true`。
    ///
    /// 同一个键位的必然按下 / 抬起交替，所以「这个键位还没记着」就是按下、「已经记着」就是抬起；聚合标志
    /// 分不清左右两只 Shift，只能这样认。
    pub fn key_event(&self, event: ModifierEvent, keys: SwitchKeys) -> bool {
        let Some(key) = event.switch.filter(|key| keys.contains(*key)) else {
            // 不是切换键、或这个键没勾上（普通字母、⌘ / ⌥、没勾的 ⇧）：作废正按着的单击
            self.cancel();
            return false;
        };
        let bit = 1u8 << event.slot.min(SLOT_BITS);
        let held = &self.held[slots_of(key)];
        let pressed = held.get() & bit == 0;
        held.set(if pressed {
            held.get() | bit
        } else {
            held.get() & !bit
        });
        // 此刻按着的切换键键位总数：两只 Shift 一起按是 2，不算裸按
        let held_count = self.count_held();
        if pressed {
            self.armed.set(if event.bare && held_count == 1 {
                Some(key)
            } else {
                None
            });
            return false;
        }
        // 抬起：还按着别的切换键键位（另一只 Shift 没松）就不算单击
        if held_count != 0 {
            self.cancel();
            return false;
        }
        if self.armed.get() == Some(key) {
            self.armed.set(None);
            true
        } else {
            false
        }
    }

    /// 普通键按下（`Shift + A` 里的 A、`⇧Tab` 里的 Tab）：作废正按着的单击，键位记录不动。
    pub fn interrupt(&self) {
        self.cancel();
    }

    /// 作废正按着的切换键：配置刚改、系统热键把组合里的第二个键截走、输入源被抢，这次抬起都不该算单击。
    pub fn cancel(&self) {
        self.armed.set(None);
    }

    /// 与系统聚合的修饰标志对账：标志里已经没有的切换键，键位整个清掉。
    ///
    /// 输入法在按住 Shift 的时候才被激活（或某个 `FlagsChanged` 被别的进程截走）会让键位一直挂着，
    /// 之后每次抬起都不再算单击；对账把那次的痕迹抹掉。
    pub fn resync(&self, shift_held: bool, control_held: bool) {
        if !shift_held {
            self.held[slots_of(SwitchKey::Shift)].set(0);
        }
        if !control_held {
            self.held[slots_of(SwitchKey::Control)].set(0);
        }
        let armed = self.armed.get();
        if armed.is_some_and(|key| self.held[slots_of(key)].get() == 0) {
            self.cancel();
        }
    }

    /// 此刻按着的切换键键位总数。
    fn count_held(&self) -> u32 {
        self.held
            .iter()
            .map(|mask| mask.get().count_ones())
            .sum::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(key: SwitchKey) -> SwitchKeys {
        SwitchKeys::NONE.with(key, true)
    }

    /// 一次裸按：`slot` 键位的按下事件。
    fn down(key: SwitchKey, slot: u8, bare: bool) -> ModifierEvent {
        ModifierEvent {
            switch: Some(key),
            slot,
            bare,
        }
    }

    /// 一次抬起。
    fn up(key: SwitchKey, slot: u8) -> ModifierEvent {
        ModifierEvent {
            switch: Some(key),
            slot,
            bare: false,
        }
    }

    /// 不是切换键的键（普通字母、⌘ / ⌥）按下与抬起：只作废单击。
    fn other() -> ModifierEvent {
        ModifierEvent {
            switch: None,
            slot: 0,
            bare: false,
        }
    }

    /// 裸按 Shift 一次算单击；同一次按下只 fire 一次。
    #[test]
    fn a_bare_shift_tap_fires_once() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        assert!(!tap.key_event(down(SwitchKey::Shift, 0, true), keys));
        assert!(tap.is_armed());
        assert!(tap.key_event(up(SwitchKey::Shift, 0), keys));
        assert!(!tap.is_armed());
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
    }

    /// 按下到抬起之间插进别的修饰键（⌥ / ⌘ 的 `FlagsChanged`）就不算单击。
    #[test]
    fn a_tap_interrupted_by_another_modifier_does_not_fire() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        assert!(tap.is_armed());
        tap.key_event(other(), keys);
        assert!(!tap.is_armed());
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
    }

    /// 按下之前已经按着别的修饰键（⌥ / ⌘ / ⌃）：那次 Shift 不是裸按，抬起不切。
    #[test]
    fn a_shift_pressed_while_a_modifier_is_held_does_not_fire() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(other(), keys); // ⌥ 按下：作废用的单击（⌥ 不是切换键）
        assert!(!tap.key_event(down(SwitchKey::Shift, 0, false), keys));
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
        assert!(!tap.key_event(other(), keys)); // ⌥ 抬起
    }

    /// 先裸按 Shift、再按 ⌘：那次 Shift 作废（`⌘⇧…` 是快捷键）。
    #[test]
    fn a_modifier_pressed_after_shift_voids_the_tap() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        tap.key_event(other(), keys); // ⌘ 按下
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
    }

    /// 左右 Shift 一起按：谁抬起都不算单击（抬起那只时另一只还按着）。
    #[test]
    fn both_shifts_held_together_never_fire() {
        let keys = only(SwitchKey::Shift).with(SwitchKey::Control, true);
        for (key, first, second) in [(SwitchKey::Shift, 0u8, 1u8), (SwitchKey::Control, 0, 1)] {
            let tap = KeyTap::default();
            assert!(!tap.key_event(down(key, first, true), keys));
            assert!(!tap.key_event(down(key, second, true), keys));
            assert!(!tap.key_event(up(key, first), keys));
            assert!(!tap.key_event(up(key, second), keys));
        }
    }

    /// 两只 Shift 分别单击（按下抬起算一次）：各切一次，不会因为上一次按过就失效。
    #[test]
    fn two_separate_shift_taps_fire_twice() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        for slot in [0u8, 1] {
            tap.key_event(down(SwitchKey::Shift, slot, true), keys);
            assert!(tap.key_event(up(SwitchKey::Shift, slot), keys), "{slot}");
        }
    }

    /// Shift 与 Control 同时按着：谁抬起都不算单击（`Ctrl + Shift` 是系统换布局的键）。
    #[test]
    fn two_different_switch_keys_never_fire() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift).with(SwitchKey::Control, true);
        tap.key_event(down(SwitchKey::Control, 0, true), keys);
        assert!(!tap.key_event(down(SwitchKey::Shift, 0, false), keys));
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
        assert!(!tap.key_event(up(SwitchKey::Control, 0), keys));
    }

    /// 只有勾上的键算单击：没勾 Shift 时它连「按下」都不记，抬起不切。
    #[test]
    fn only_checked_keys_fire() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Control);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
        tap.key_event(down(SwitchKey::Control, 0, true), keys);
        assert!(tap.key_event(up(SwitchKey::Control, 0), keys));
    }

    /// 一个切换键都没勾：修饰键单击都不算。
    #[test]
    fn no_checked_key_fires_nothing() {
        let tap = KeyTap::default();
        for key in [SwitchKey::Shift, SwitchKey::Control] {
            tap.key_event(down(key, 0, true), SwitchKeys::NONE);
            assert!(!tap.key_event(up(key, 0), SwitchKeys::NONE), "{key:?}");
        }
    }

    /// `interrupt` 与改配置都能作废正按着的单击。
    #[test]
    fn interrupt_and_cancel_drop_the_pending_tap() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        tap.interrupt();
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));

        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        tap.cancel();
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
    }

    /// 对账：抬起事件漏了（输入法是按住 Shift 才激活的）时，标志里已经没有 Shift 就把键位清掉，
    /// 下一次裸按 Shift 仍然算单击。
    #[test]
    fn resync_clears_slots_the_system_no_longer_reports() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        tap.key_event(down(SwitchKey::Shift, 1, true), keys); // 抬起漏了：两只都挂着
        assert!(!tap.key_event(up(SwitchKey::Shift, 0), keys));
        assert!(!tap.key_event(up(SwitchKey::Shift, 1), keys));
        tap.resync(false, false);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        assert!(tap.key_event(up(SwitchKey::Shift, 0), keys));
    }

    /// 对账不能反过来误伤：标志里还有 Shift（正按着）时不清键位，抬起照常算单击。
    #[test]
    fn resync_keeps_a_slot_the_system_still_reports() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        tap.resync(true, false);
        assert!(tap.key_event(up(SwitchKey::Shift, 0), keys));
    }

    /// 作废之后重新裸按一次仍然有效（状态干净，不留幽灵按下）。
    #[test]
    fn a_fresh_tap_works_after_a_discarded_one() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        tap.key_event(other(), keys);
        tap.key_event(up(SwitchKey::Shift, 0), keys);
        tap.key_event(down(SwitchKey::Shift, 0, true), keys);
        assert!(tap.key_event(up(SwitchKey::Shift, 0), keys));
    }
}
