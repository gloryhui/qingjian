//! 中 / 英切换：修饰键单击的判定与切模式后的收尾。
//!
//! 判定本身在 [`crate::host::ModeState`]（纯状态机，可单测），这里只做「事件 → 按下 / 抬起」和收尾。

use objc2::sel;
use objc2_app_kit::{NSEvent, NSEventModifierFlags};
use objc2_foundation::NSTimer;
use qingjian_platform::SwitchKey;

use super::QingjianInputController;
use crate::host;
use crate::imk::{TextClient, modifiers};

impl QingjianInputController {
    /// 给先到的 Shift 抬起、后到的组合键 KeyDown 留一段事件交付余量。
    const SWITCH_GRACE_SECONDS: f64 = 0.15;

    /// 一次修饰键按下 / 抬起（`FlagsChanged`）：单击先待定，等迟到的组合键 KeyDown 作废。
    ///
    /// 一律返回 false：修饰键本身要交给应用，`⇧ + 字母`、`⌘ + Tab` 照常，切换模式也不改变这一键的归属。
    pub(super) fn handle_flags(&self, event: &NSEvent, client: TextClient<'_>) -> bool {
        let flags = event.modifierFlags();
        let modifier = modifiers::modifier_event(event);
        // 连续裸按两次 Shift 时，第二次按下应先确认第一次，不能覆盖第一次的待定状态。
        let another_press = modifier.switch.is_some_and(|key| match key {
            SwitchKey::Shift => flags.contains(NSEventModifierFlags::Shift),
            SwitchKey::Control => flags.contains(NSEventModifierFlags::Control),
            SwitchKey::CtrlAltSpace => false,
        });
        if another_press {
            let flipped = host::with(|h| {
                if !h.mode.has_pending() {
                    return false;
                }
                if let Some(timer) = h.pending_switch.take() {
                    timer.invalidate();
                }
                h.mode.confirm_pending()
            })
            .unwrap_or(false);
            if flipped {
                self.switch_language(client);
            }
        }
        let pending = host::with(|h| {
            let pending = h.mode.modifier_event(modifier);
            // 键位的开合是自己记的（聚合标志分不清左右两只），记漏了就照聚合标志清掉：
            // 输入法可能在按住 Shift 的时候才被激活。必须在判定之后对账，否则抬起会被当成按下。
            h.mode.resync(
                flags.contains(NSEventModifierFlags::Shift),
                flags.contains(NSEventModifierFlags::Control),
            );
            if pending {
                if let Some(timer) = h.pending_switch.take() {
                    timer.invalidate();
                }
                h.pending_switch = Some(unsafe {
                    NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                        Self::SWITCH_GRACE_SECONDS,
                        self,
                        sel!(confirmPendingSwitch:),
                        Some(client.object()),
                        false,
                    )
                });
            }
            pending
        })
        .unwrap_or(false);
        tracing::debug!(key_code = event.keyCode(), ?flags, pending, "修饰键事件");
        false
    }

    /// 翻了中 / 英模式：把没打完的东西收干净，不带进新模式，也不留幽灵状态。
    ///
    /// 手上没打完的拼音先原样上屏（留着会变成英文模式里的幽灵文本）；翻译窗口开着就整个放弃
    /// （`end_translation` 顺带停联想、清会话、收窗口，切完模式再按回车不该替换选区）；
    /// 状态项立刻跟上。
    pub(super) fn switch_language(&self, client: TextClient<'_>) {
        self.commit_raw(client);
        let english = host::with(|h| {
            h.engine.set_english_mode(false);
            h.end_translation();
            h.cancel_prediction();
            h.window.hide();
            h.indicator.update(h.mode.english());
            h.mode.english()
        });
        tracing::info!(?english, "切换中英模式");
    }
}
