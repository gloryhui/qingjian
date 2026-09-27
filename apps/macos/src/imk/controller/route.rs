//! 一个按下事件在青简这里的归属顺序：纯直通 → 确认译文 → 翻译快捷键 → 其余照常处理。
//!
//! 顺序在这里是一个真判定而不是 `dispatch_event` 里的写法：英文模式的纯直通必须整个赢过翻译路径，
//! 否则「纯英文」还留着拦快捷键的青简逻辑。切模式时也会清掉待确认的译文，直通与它同时成立只是兜底。

/// `Route::decide` 的结果。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// 纯直通：整个键交给应用，不进引擎、不组句、不查候选，翻译路径一律不拦。
    Passthrough,

    /// 有译文在等确认：这一键先给确认流程（回车替换、Esc 保留原文）。
    Review,

    /// 命中翻译选中文字的快捷键：读选区交给云端。
    Translate,

    /// 都不命中：继续走选词数字、命令键与文本路径。
    Continue,
}

impl Route {
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
mod tests {
    use super::Route;

    /// 纯直通赢过一切翻译状态：这一模式下青简除了中英切换什么都不做。
    #[test]
    fn pure_passthrough_beats_every_translation_path() {
        for reviewing in [false, true] {
            for hotkey in [false, true] {
                assert_eq!(
                    Route::decide(true, reviewing, hotkey),
                    Route::Passthrough,
                    "reviewing={reviewing} hotkey={hotkey}"
                );
            }
        }
    }

    /// 不直通时才轮到翻译路径：确认译文先于快捷键（快捷键命中也不该丢着等确认的译文不管）。
    #[test]
    fn review_and_hotkey_only_apply_when_not_passthrough() {
        assert_eq!(Route::decide(false, true, true), Route::Review);
        assert_eq!(Route::decide(false, true, false), Route::Review);
        assert_eq!(Route::decide(false, false, true), Route::Translate);
    }

    /// 三格都不成立才交给后面的选词与文本路径。
    #[test]
    fn an_ordinary_key_continues_to_the_engine() {
        assert_eq!(Route::decide(false, false, false), Route::Continue);
    }
}
