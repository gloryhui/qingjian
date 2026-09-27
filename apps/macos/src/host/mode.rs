//! 中英模式：单击切换键翻转的持久状态，以及「这一键整个交给应用」的纯直通判定。
//!
//! 与 Windows Server 的 `Router.english`、Linux Server 的 `SessionInfo.english` 是同一维度：模式是
//! 输入法自己的一份状态，**不是硬件 Caps Lock 的开关位置**。Caps Lock 只管大小写——亮着时字母以大写
//! 送来，青简按普通键盘的样子把它交给应用，既不改中英模式，也不另起一套行为。

use qingjian_platform::key_tap::KeyTap;
use qingjian_platform::{SwitchKey, SwitchKeys};

/// 中英模式状态：配置（`[general] english_mode`、`[shortcut] switch_mode`）套进来后才可能切到英文。
#[derive(Default)]
pub struct ModeState {
    /// 当前是否英文模式。进程级一份，所有应用共用（切窗口、新打开的应用都跟着走）。
    english: bool,

    /// 正按着哪个切换键、之后没插进别的键（见 [`KeyTap`]）。
    tap: KeyTap,

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
    }

    /// 当前中英模式（输入法自己记的那一份，与 Caps Lock 亮没亮无关）。
    pub fn english(&self) -> bool {
        self.english
    }

    /// 一次按键按下：`switch` 是该键对应的切换键，普通键与别的修饰键传 `None`（它会作废单击）。
    pub fn key_down(&self, switch: Option<SwitchKey>, repeat: bool) {
        self.tap.key_down(switch, repeat, self.switch_keys);
    }

    /// 一次按键抬起：命中单击切换键就翻中 / 英，返回是否翻了（调用方据此清理组句）。
    /// 关着内置英文模式时那次单击作废，返回 `false`。
    pub fn key_up(&mut self, switch: Option<SwitchKey>) -> bool {
        if !self.tap.key_up(switch, self.switch_keys) {
            return false;
        }
        self.toggle()
    }

    /// 翻转中 / 英模式。关掉内置英文模式时不翻，返回 `false`。
    pub fn toggle(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.english = !self.english;
        true
    }

    /// 这一键是不是整个交给应用（纯直通）：不进引擎、不建组句、不产生 marked text、不转全角标点。
    ///
    /// 只认中英模式这一份状态：英文模式且这个应用不给英文候选才直通。`candidates_in_app` 为
    /// `[general] english_candidates` 开着且这个应用不在 `[apps] english_candidates_off` 里
    /// （见 [`Host::english_candidates_in`]）。组句中一律 `false`：那一键仍走文本路径，
    /// 由它先把缓冲区原样上屏再放行。Caps Lock 不参与判定——它只管大小写。
    ///
    /// [`Host::english_candidates_in`]: crate::host::Host::english_candidates_in
    pub fn passthrough(&self, candidates_in_app: bool, composing: bool) -> bool {
        self.english && !candidates_in_app && !composing
    }
}

#[cfg(test)]
mod tests {
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

    /// 单击 Shift：中文 → 英文，再单击：英文 → 中文。
    #[test]
    fn a_shift_tap_toggles_both_ways() {
        let mut mode = mode();
        assert!(!mode.english());
        mode.key_down(Some(SwitchKey::Shift), false);
        assert!(mode.key_up(Some(SwitchKey::Shift)));
        assert!(mode.english());

        mode.key_down(Some(SwitchKey::Shift), false);
        assert!(mode.key_up(Some(SwitchKey::Shift)));
        assert!(!mode.english());
    }

    /// `Shift + 任何别的键` 都不切换：普通字母、数字、Tab（壳里都映射不成切换键，即 `None`）、
    /// 以及别的修饰键（这里是按着 Shift 又按 Ctrl）。
    #[test]
    fn shift_with_another_key_never_toggles() {
        for interrupt in [None, Some(SwitchKey::Control)] {
            let mut mode = mode();
            mode.key_down(Some(SwitchKey::Shift), false);
            mode.key_down(interrupt, false);
            assert!(!mode.key_up(Some(SwitchKey::Shift)), "{interrupt:?}");
            assert!(!mode.english(), "{interrupt:?}");
        }
    }

    /// 按住 Shift 连打几个键再抬起：中间每一下都作废单击，一次都不切。
    #[test]
    fn holding_shift_across_several_keys_does_not_toggle() {
        let mut mode = mode();
        mode.key_down(Some(SwitchKey::Shift), false);
        for _ in 0..5 {
            mode.key_down(None, false);
            assert!(!mode.key_up(None));
        }
        assert!(!mode.key_up(Some(SwitchKey::Shift)));
        assert!(!mode.english());
    }

    /// 没勾 Shift 时按它不切换（`[shortcut] switch_mode = []`）。
    #[test]
    fn an_unchecked_key_does_not_toggle() {
        let mut mode = ModeState::default();
        mode.set_settings(SwitchKeys::NONE, true);
        mode.key_down(Some(SwitchKey::Shift), false);
        assert!(!mode.key_up(Some(SwitchKey::Shift)));
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
        mode.key_down(Some(SwitchKey::Shift), false);
        assert!(!mode.key_up(Some(SwitchKey::Shift)));
        assert!(!mode.english());
    }

    /// 改配置作废正按着的 Shift：切换键刚换掉，那次按下不该算单击。
    #[test]
    fn changing_settings_drops_the_pending_tap() {
        let mut mode = mode();
        mode.key_down(Some(SwitchKey::Shift), false);
        mode.set_settings(shift(), true);
        assert!(!mode.key_up(Some(SwitchKey::Shift)));
    }

    /// 英文模式没候选 = 纯直通：字母、数字、标点都不该被青简接走。
    #[test]
    fn english_mode_without_candidates_is_pure_passthrough() {
        let mut mode = mode();
        assert!(mode.toggle());
        assert!(mode.passthrough(false, false));
        // 显式开了英文候选才组英文句
        assert!(!mode.passthrough(true, false));
    }

    /// 中文模式照常组句：直通判定只认英文模式这一份状态，没在组句才整个放行。
    #[test]
    fn chinese_mode_always_goes_to_the_engine() {
        let mode = mode();
        assert!(!mode.passthrough(false, false));
        assert!(!mode.passthrough(true, false));
        // 组句中即使切了模式也先把这一键交给文本路径处理（那里先上屏再放行）
        assert!(!mode.passthrough(true, true));
    }

    /// Caps Lock 只管大小写：它按 `FlagsChanged` 送来，映射不成切换键（`None`），
    /// 亮灭多少下都不翻中英模式——模式只有单击切换键这一个来源。
    #[test]
    fn caps_lock_presses_never_change_the_mode() {
        let mut mode = mode();
        for _ in 0..4 {
            mode.key_down(None, false);
            assert!(!mode.key_up(None));
        }
        assert!(!mode.english());
        // 已经在英文模式时也一样：Caps Lock 切不回中文
        mode.key_down(Some(SwitchKey::Shift), false);
        assert!(mode.key_up(Some(SwitchKey::Shift)));
        for _ in 0..4 {
            mode.key_down(None, false);
            assert!(!mode.key_up(None));
        }
        assert!(mode.english());
    }
}
