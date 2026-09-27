//! 中 / 英切换：修饰键单击的判定与切模式后的收尾。
//!
//! 判定本身在 [`crate::host::ModeState`]（纯状态机，可单测），这里只做「事件 → 按下 / 抬起」和收尾。

use super::*;

impl QingjianInputController {
    /// 一次修饰键按下 / 抬起（`FlagsChanged`）：喂给单击状态机，抬起时命中单击就翻中 / 英。
    ///
    /// 一律返回 false：修饰键本身要交给应用，`⇧ + 字母`、`⌘ + Tab` 照常，切换模式也不改变这一键的归属。
    pub(super) fn handle_flags(&self, event: &NSEvent, client: TextClient<'_>) -> bool {
        match modifiers::switch_key_of(event.keyCode()) {
            Some(key) => {
                let flipped = host::with(|h| {
                    if modifiers::switch_key_down(event, key) {
                        h.mode.key_down(Some(key), event.isARepeat());
                        return false;
                    }
                    h.mode.key_up(Some(key))
                })
                .unwrap_or(false);
                if flipped {
                    self.switch_language(client);
                }
            }
            // 别的修饰键（⌘ / ⌥ / Caps Lock）按下：作废正按着的切换键，`⌘ + Shift` 之后抬起 Shift 不算单击。
            // 抬起不用管：按下那一下已经作废过了。
            None => {
                host::with(|h| h.mode.key_down(None, false));
            }
        }
        false
    }

    /// 翻了中 / 英模式：手上没打完的拼音先原样上屏（留着会变成英文模式里的幽灵文本），
    /// 候选窗口与在飞的联想起收掉，状态项立刻跟上。
    fn switch_language(&self, client: TextClient<'_>) {
        self.commit_raw(client);
        let english = host::with(|h| {
            h.engine.set_english_mode(false);
            h.cancel_prediction();
            h.window.hide();
            h.indicator.update(h.mode.english());
            h.mode.english()
        });
        tracing::debug!(?english, "切换中英模式");
    }
}
