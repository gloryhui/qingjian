//! 可打印字符的处理：中英文模式、直输段、表达式与问字模式的分流。

use super::*;

/// 这个大写字母是不是**按着 Shift** 打的（只有它归 `[general] shift_letter` 管）。
///
/// Caps Lock 亮着送来的大写只是大写锁定：勾了 `shift_letter = "compose"` 就把它收进中文缓冲区，
/// 等于让 ⇪ 决定要不要组句——那键只管大小写。这种大写整个交给应用，一个字都不进青简。
fn shifted_uppercase(c: char, modifiers: Modifiers) -> bool {
    c.is_ascii_uppercase() && modifiers.shift
}

/// 英文模式下的 ASCII 标点始终按半角交给应用，即使本次按键绕到通用文本处理的末尾。
fn english_punctuation_is_passthrough(english: bool, c: char) -> bool {
    english && c.is_ascii_punctuation()
}

impl QingjianInputController {
    /// `modifiers` 是这一次按键按着的修饰键：只有真正按着 Shift 的大写才走 `shift_letter` 那条策略。
    pub(super) fn handle_text(
        &self,
        text: &str,
        client: TextClient<'_>,
        modifiers: Modifiers,
    ) -> bool {
        tracing::debug!(%text, "inputText");
        let mut composing = host::with(|h| !h.engine.composition().is_empty()).unwrap_or(false);
        // 中英模式是输入法自己记的那一份（单击 Shift 翻），Caps Lock 只管大小写
        let english = host::with(|h| h.mode.english()).unwrap_or(false);
        // 显式开了英文候选才有英文词；终端、编辑器这类应用（`[apps] english_candidates_off`）里不给
        let english_candidates = english
            && host::with(|h| h.english_candidates_in(client.bundle_identifier().as_deref()))
                .unwrap_or(false);
        // 英文候选组词中候选又关了（改了配置或换到不给候选的应用）：敲的字母先原样上屏，别把它们当拼音
        if composing
            && !english_candidates
            && host::with(|h| h.engine.english_mode()).unwrap_or(false)
        {
            self.commit_raw(client);
            composing = false;
        }
        let [byte] = text.as_bytes() else {
            // 多字符文本（如输入法联动、粘贴）：先把当前候选（英文模式下是敲的字母）上屏，再交给应用
            if composing {
                if host::with(|h| h.engine.english_mode()).unwrap_or(false) {
                    self.commit_raw(client);
                } else {
                    self.commit_highlighted(client);
                }
            }
            return false;
        };
        let c = char::from(*byte);
        // 缓冲区为空时敲 ? 先进问字模式（配置 `[shortcut] question_mark`，缺省关），中英文模式都行：
        // 后面跟字母就是在问字，跟别的键就还原成问号
        if !composing
            && c == QUESTION_PREFIX
            && host::with(|h| h.engine.takes_question_mark()).unwrap_or(false)
        {
            host::with(|h| h.engine.push(c));
            self.refresh(client);
            return true;
        }
        // 双拼下 Shift+V / Shift+U 进表达式 / 问字模式（全拼下的 v / u 被音节占了）
        if !composing && !english && host::with(|h| h.engine.takes_mode_letter(c)).unwrap_or(false)
        {
            host::with(|h| h.engine.push(c));
            self.refresh(client);
            return true;
        }
        let question = composing && host::with(|h| h.engine.question_mode()).unwrap_or(false);
        // 英文模式下问字：字母以大写送来（按着 Shift 或 Caps Lock 亮着），按小写收进问题
        let c = if question && english && c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else {
            c
        };
        host::with(|h| h.engine.set_english_mode(english_candidates && !question));
        let (page_previous, page_next) =
            host::with(|h| h.page_keys).unwrap_or(qingjian_platform::DEFAULT_PAGE_KEYS);
        // 英文模式且不给英文候选 = 纯直通，跟系统 ABC 键盘一样：这一键整个交给应用，青简不接、不组句、
        // 不转全角标点。没在组句的键在 `dispatch_event` 里已经整个放行，走到这里只可能是组句中途
        // 候选又关了，先把敲的原样上屏，别留拼音幽灵文本；问字模式是显式按出来的，不收在这一步。
        if english && !english_candidates && !question {
            if composing {
                self.commit_raw(client);
                host::with(|h| h.engine.note_passthrough(c));
            }
            return false;
        }
        // 英文候选：字母（以及组词中的 _ ' -）进缓冲区，候选来自英文词表。选词与中文模式一样：
        // 空格选高亮（词上屏后空格照样交给应用，接着打下一个词）、数字选当前页第 N 个、翻页键翻页；
        // 数字对应的格子没有候选（kubectl 这类词表没有的词、候选不足 N 个）时是标识符的一部分（foo1）。
        // 回车、标点先把敲的字母原样上屏再交给应用
        if english && !question {
            // 按着 Shift 打的大写进缓冲区（词表里有 `GitHub` 这类）；Caps Lock 只管大小写，
            // 亮着时字母不分按没按 Shift 都以大写送来，收回小写去匹配词表
            let letter = if modifiers::caps_lock_on() {
                c.to_ascii_lowercase()
            } else {
                c
            };
            if composing
                && let Some(offset) = c.to_digit(10).filter(|d| *d > 0)
                && let Some(index) =
                    host::with(|h| h.session.index_on_page(offset as usize - 1)).flatten()
            {
                return self.commit_index(index, client);
            }
            if c.is_ascii_alphabetic()
                || (composing && (c.is_ascii_digit() || matches!(c, '_' | '\'' | '-')))
            {
                host::with(|h| h.engine.push(letter));
                self.refresh(client);
                return true;
            }
            if composing && c == page_previous {
                return self.turn_page(-1, client);
            }
            if composing && c == page_next {
                return self.turn_page(1, client);
            }
            if composing {
                if c == ' ' {
                    self.commit_highlighted(client);
                } else {
                    self.commit_raw(client);
                }
            }
            host::with(|h| h.engine.note_passthrough(c));
            return false;
        }
        // 表达式模式（v 开头）：数字与运算符进缓冲区，不当选词 / 翻页键
        let expression = composing && host::with(|h| h.engine.expression_mode()).unwrap_or(false);
        // 英文直输段（缓冲区里已有 `-` 这类字符）：可见字符一律追加，空格 / 回车整段原样上屏
        let raw = composing && host::with(|h| h.engine.raw_mode()).unwrap_or(false);
        // 组句中敲 `-`：进入英文直输段（`no-way`）；配成翻页键（`[general] page_keys` 选 `-=`）时才翻页
        let hyphen = composing && !question && c == '-' && c != page_previous && c != page_next;
        // 问字模式下敲的还可能是码点（`u4e00`、`u+1f600`）：数字与 `+` 进缓冲区而不是选词
        let unicode = question && host::with(|h| h.engine.unicode_entry()).unwrap_or(false);
        // 微软 / 搜狗双拼的 `;` 是 ing 键：末尾有落单声母时进缓冲区，其他时候还是标点
        let semicolon =
            composing && c == ';' && host::with(|h| h.engine.takes_semicolon()).unwrap_or(false);
        // 组句中敲半角标点：进缓冲区，整段成为英文直输段（`hello,` `dui'ma?`），中文模式下也能打带标点的英文；
        // 翻页键除外；⇧+数字（! @ # …）在前面已被删候选 / 译词键截走
        let punctuation = composing
            && !question
            && !expression
            && c.is_ascii_punctuation()
            && c != page_previous
            && c != page_next;
        if c.is_ascii_lowercase()
            || (composing && c == '\'')
            || semicolon
            || (expression && qingjian_core::shortcut::is_expression_char(c))
            || (raw && c.is_ascii_graphic())
            || (unicode && (c.is_ascii_digit() || c == '+'))
            || hyphen
            || punctuation
        {
            host::with(|h| h.engine.push(c));
            self.refresh(client);
            return true;
        }
        // 直输段里的空格：整段原样上屏，空格本身也交给应用（`hello, world` 里的空格要在）
        if raw && c == ' ' {
            self.commit_highlighted(client);
            host::with(|h| h.engine.note_passthrough(c));
            return false;
        }
        if composing && self.restore_bare_question(client) {
            // 空格只是「把这个 ? 上屏」，不再多打一个空格；其他键按非组句状态继续处理
            if c == ' ' {
                return true;
            }
            return self.handle_text(text, client, modifiers);
        }
        // 按住 Shift 打的大写字母：缺省是临时打英文，先把拼音原样上屏，再把字母交给应用；
        // `[general] shift_letter = "compose"` 时进缓冲区（Core 按小写匹配、原样上屏时还原大写）。
        // ⇪ 亮着送来的大写不参与这条策略，一律整个交给应用（见 [`shifted_uppercase`]）
        if c.is_ascii_uppercase() {
            if shifted_uppercase(c, modifiers)
                && host::with(|h| h.engine.shift_letter_compose()).unwrap_or(false)
            {
                host::with(|h| h.engine.push(c));
                self.refresh(client);
                return true;
            }
            if composing {
                self.commit_raw(client);
            }
            host::with(|h| h.engine.note_passthrough(c));
            return false;
        }
        if composing {
            match c {
                ' ' => return self.commit_highlighted(client),
                '1'..='9' => {
                    let offset = usize::from(*byte - b'1');
                    if let Some(index) = host::with(|h| h.session.index_on_page(offset)).flatten() {
                        return self.commit_index(index, client);
                    }
                    // 这一页没有这一格（`gpt6` 只有三个候选）：数字当内容进缓冲区，成为英文直输段；
                    // 问字模式里数字不是问题的一部分，不算
                    if !question {
                        host::with(|h| h.engine.push(c));
                        self.refresh(client);
                    }
                    return true;
                }
                c if c == page_previous => return self.turn_page(-1, client),
                c if c == page_next => return self.turn_page(1, client),
                // 其他字符：把当前高亮候选上屏，再按非组句状态处理这个字符
                _ => {
                    self.commit_highlighted(client);
                }
            }
        }
        // 英文模式任何路径都不能误入中文全角转换（例如英文候选组句结束时的括号）。
        // 中文模式下照常转全角；其他字符原样交给应用。
        let punctuated = if english_punctuation_is_passthrough(english, c) {
            None
        } else {
            host::with(|h| h.engine.punctuate(c)).flatten()
        };
        match punctuated {
            Some(full_width) => {
                client.insert_text(full_width);
                true
            }
            None => {
                host::with(|h| h.engine.note_passthrough(c));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifiers(shift: bool) -> Modifiers {
        Modifiers {
            option: false,
            shift,
            control: false,
            command: false,
        }
    }

    /// 按着 Shift 的大写才算 Shift 输入；小写不算（它照常进组句，跟 `shift_letter` 无关）。
    #[test]
    fn only_a_shift_held_uppercase_counts() {
        assert!(shifted_uppercase('A', modifiers(true)));
        assert!(!shifted_uppercase('A', modifiers(false)));
        for shift in [true, false] {
            assert!(!shifted_uppercase('a', modifiers(shift)), "{shift}");
            assert!(!shifted_uppercase('1', modifiers(shift)), "{shift}");
        }
    }

    /// Caps Lock 亮着（没按 Shift）送来的大写不参与 `shift_letter`：那种大写只是大写锁定，
    /// 进了组句就成了拼音里的幽灵字母。
    #[test]
    fn caps_lock_uppercase_is_not_a_shift_letter() {
        assert!(!shifted_uppercase('C', modifiers(false)), "⇪ 亮着打的大写");
        assert!(
            shifted_uppercase('C', modifiers(true)),
            "⇧ + C 才是 Shift 输入"
        );
    }

    /// 英文模式的括号等 ASCII 标点不能走中文全角转换；中文模式仍保留原有转换路径。
    #[test]
    fn english_punctuation_stays_ascii() {
        for c in ['(', ')', '[', ']', ',', '.', '?', '!'] {
            assert!(english_punctuation_is_passthrough(true, c), "{c}");
            assert!(!english_punctuation_is_passthrough(false, c), "{c}");
        }
        assert!(!english_punctuation_is_passthrough(true, 'a'));
    }
}
