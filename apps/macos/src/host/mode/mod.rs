//! 中英模式：单击切换键翻转的持久状态，以及「这一键整个交给应用」的纯直通判定。
//!
//! 与 Windows Server 的 `Router.english`、Linux Server 的 `SessionInfo.english` 是同一维度：模式是
//! 输入法自己的一份状态，**不是硬件 Caps Lock 的开关位置**。Caps Lock 只管大小写——亮着时字母以大写
//! 送来，青简按普通键盘的样子把它交给应用，既不改中英模式，也不另起一套行为。

use std::cell::Cell;
use std::time::{Duration, Instant};

use qingjian_platform::key_tap::{KeyTap, ModifierEvent};
use qingjian_platform::{SwitchKey, SwitchKeys};

/// 只有按下到抬起不超过这个时长，才算一次「单击」；与抬起后的乱序保护时间无关。
const SWITCH_TAP_MAX_DURATION: Duration = Duration::from_millis(500);

/// 中英模式状态：配置（`[general] english_mode`、`[shortcut] switch_mode`）套进来后才可能切到英文。
#[derive(Default)]
pub struct ModeState {
    /// 当前是否英文模式。进程级一份，所有应用共用（切窗口、新打开的应用都跟着走）。
    english: bool,

    /// 正按着哪些物理键位、哪一次按下还没被别的键打断（见 [`KeyTap`]）。
    tap: KeyTap,

    /// 当前尚未被组合键打断的那次按下时间；取消单击时必须同步清掉。
    pressed_at: Cell<Option<Instant>>,

    /// Shift 抬起后暂存单击；给延迟到达的组合键 KeyDown 一次作废机会。
    pending: Option<SwitchKey>,

    /// 勾了哪些单击切换键（配置 `[shortcut] switch_mode`）。
    switch_keys: SwitchKeys,

    /// 内置英文模式开关（配置 `[general] english_mode`）。关着时切换键不再切到英文。
    enabled: bool,
}

impl ModeState {
    /// 套配置。关掉内置英文模式时一并退回中文（与 Windows `apply_mode_settings` 同一处理），
    /// 并作废正按着的切换键：配置刚改，那次按下不该算单击。
    pub fn set_settings(&mut self, switch_keys: SwitchKeys, enabled: bool) {
        self.switch_keys = switch_keys;
        self.enabled = enabled;
        if !enabled {
            self.english = false;
        }
        self.tap.cancel();
        self.pressed_at.set(None);
        self.pending = None;
    }

    /// 当前中英模式（输入法自己记的那一份，与 Caps Lock 亮没亮无关）。
    pub fn english(&self) -> bool {
        self.english
    }

    /// 一次物理修饰键按下 / 抬起：命中单击先待定，返回是否需要安排确认定时器。
    /// 有些客户端的组合键 KeyDown 比 Shift 抬起事件晚到，不能在抬起时直接切模式。
    pub fn modifier_event(&mut self, event: ModifierEvent) -> bool {
        self.modifier_event_at(event, Instant::now())
    }

    /// 可注入单调时间的判定内核，供状态机测试精确覆盖时长边界。
    fn modifier_event_at(&mut self, event: ModifierEvent, now: Instant) -> bool {
        // 抬起后的等待期内又出现修饰键事件，先作废旧单击；下一次独立短按由控制器另行确认。
        self.pending = None;
        let pressed_at = self.pressed_at.take();
        let tapped = self.tap.key_event(event, self.switch_keys);
        if self.tap.is_armed() {
            self.pressed_at.set(pressed_at.or(Some(now)));
        }
        if !tapped {
            return false;
        }
        // 恰好 500ms 仍算短按；超过才是长按。时钟倒退不应制造一次单击。
        if !pressed_at.is_some_and(|start| {
            now.checked_duration_since(start)
                .is_some_and(|duration| duration <= SWITCH_TAP_MAX_DURATION)
        }) {
            self.pending = None;
            return false;
        }
        if !self.enabled {
            return false;
        }
        self.pending = event.switch;
        true
    }

    /// 下一个 KeyDown 若仍带着刚抬起的切换键，就是延迟到达的组合键，作废单击；
    /// 否则先确认单击，确保紧接着输入的普通键按新模式处理。
    pub fn key_down(&mut self, shift: bool, control: bool) -> bool {
        self.interrupt();
        let pending = self.pending.take();
        if pending.is_some_and(|key| match key {
            SwitchKey::Shift => shift,
            SwitchKey::Control => control,
            SwitchKey::CtrlAltSpace => false,
        }) {
            return false;
        }
        pending.is_some() && self.toggle()
    }

    /// 没有后续 KeyDown 时，到期确认裸按；切换后的组句收尾由调用方完成。
    pub fn confirm_pending(&mut self) -> bool {
        self.pending.take().is_some() && self.toggle()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 输入源已停用，待定单击不能在旧客户端上完成。
    pub fn cancel_pending(&mut self) {
        self.pending = None;
        self.tap.cancel();
        self.pressed_at.set(None);
    }

    /// 普通键按下（`Shift + A` 里的 A、`⇧Tab` 里的 Tab）：正按着的那次单击作废，模式不动。
    pub fn interrupt(&self) {
        self.tap.interrupt();
        self.pressed_at.set(None);
    }

    /// 与系统聚合的修饰标志对账，抹掉抬起事件漏了的键位（见 [`KeyTap::resync`]）。
    pub fn resync(&self, shift_held: bool, control_held: bool) {
        self.tap.resync(shift_held, control_held);
        if !self.tap.is_armed() {
            self.pressed_at.set(None);
        }
    }

    /// 翻转中 / 英模式。关掉内置英文模式时不翻，返回 `false`。
    pub fn toggle(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.english = !self.english;
        true
    }

    /// 英文模式始终直通，候选配置、应用和组句状态都不参与判定。
    /// 未完成的中文组句由切换事件收尾，不能等下一次 KeyDown 才处理。
    pub fn passthrough(&self) -> bool {
        self.english
    }
}

#[cfg(test)]
mod tests;
