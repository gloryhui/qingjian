//! 一个按下事件在青简这里的归属顺序：纯直通 → 确认译文 → 翻译快捷键 → 其余照常处理。
//!
//! 顺序在这里是一个真判定而不是 `dispatch_event` 里的写法：英文模式的纯直通必须整个赢过翻译路径，
//! 否则「纯英文」还留着拦快捷键的青简逻辑。切模式时也会清掉待确认的译文，直通与它同时成立只是兜底。

use crate::host::ModeState;

/// 按键路由的结果。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// 纯直通：整个事件交还应用，不读字符、不上屏、不组句、不查候选、不走翻译。
    Passthrough,

    /// 有译文在等确认：这一键先给确认流程（回车替换、Esc 保留原文）。
    Review,

    /// 命中翻译选中文字的快捷键：读选区交给云端。
    Translate,

    /// 都不命中：继续走选词数字、命令键与文本路径。
    Continue,
}

impl Route {
    /// KeyDown 的第一道门：只接触模式状态，英文路径不依赖字符、应用、候选或 Engine。
    pub(super) fn key_down(mode: &ModeState) -> Self {
        mode.interrupt();
        Self::decide(mode.passthrough(), false, false)
    }

    /// 按顺序判定这一键归谁：三个条件由调用方算好，直通排第一。
    pub(super) fn decide(passthrough: bool, reviewing: bool, hotkey: bool) -> Self {
        if passthrough {
            Self::Passthrough
        } else if reviewing {
            Self::Review
        } else if hotkey {
            Self::Translate
        } else {
            Self::Continue
        }
    }
}

#[cfg(test)]
mod tests;
