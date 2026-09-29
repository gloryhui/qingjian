//! IMK 输入控制器：每个输入会话（每个应用的文本框）一个实例。
//!
//! 只做两件事：把按键翻译成 Engine 的调用，把 Engine 返回的候选交给候选窗口。
//! **这里不允许出现排序、词库或翻译逻辑。** 会话状态（候选、高亮、页码）在 [`crate::host::Session`]。

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSMenu};
use objc2_foundation::{NSObjectProtocol, NSTimer};
use objc2_input_method_kit::{IMKInputController, IMKServer};
use qingjian_core::Candidate;
use qingjian_platform::Modifiers;

use self::route::Route;
use super::{TextClient, catch_panic, recover_from_panic, secure_input};
use crate::candidates::Preedit;
use crate::host;
use crate::menubar;

mod command;
mod commit;
mod display;
mod mode;
mod route;
mod text;
mod translate;

#[cfg(test)]
mod tests;

/// 订阅修饰键后 IMK 不再提供默认的鼠标提交，因此鼠标按下也由控制器收尾并放行。
const INPUT_EVENTS: NSEventMask = NSEventMask::KeyDown
    .union(NSEventMask::FlagsChanged)
    .union(NSEventMask::LeftMouseDown)
    .union(NSEventMask::RightMouseDown)
    .union(NSEventMask::OtherMouseDown);

define_class!(
    // SAFETY:
    // - IMKInputController 允许子类化，Apple 文档的标准用法就是继承它。
    // - 没有实现 Drop。
    #[unsafe(super(IMKInputController))]
    // 名字要和 Info.plist 的 InputMethodServerControllerClass 一致
    #[name = "QingjianInputController"]
    #[ivars = ()]
    pub struct QingjianInputController;

    impl QingjianInputController {
        /// IMK 默认只发送 KeyDown；必须显式订阅 FlagsChanged 才能收到 Shift 按下 / 抬起。
        #[unsafe(method(recognizedEvents:))]
        fn recognized_events(&self, _sender: Option<&AnyObject>) -> usize {
            INPUT_EVENTS.bits() as usize
        }

        /// IMKServer 为每个新会话调用的指定初始化方法，在这里放好 ivars。
        #[unsafe(method_id(initWithServer:delegate:client:))]
        fn init_with_server(
            this: Allocated<Self>,
            server: Option<&IMKServer>,
            delegate: Option<&AnyObject>,
            client: Option<&AnyObject>,
        ) -> Option<Retained<Self>> {
            tracing::info!("新建输入会话");
            let this = this.set_ivars(());
            unsafe { msg_send![super(this), initWithServer: server, delegate: delegate, client: client] }
        }

        /// 所有按键事件都到这里（IMK 第一层协议）。IMK 按控制器实现了哪一层决定路线，实现了这个方法就不会再分发成
        /// `inputText:client:` / `didCommandBySelector:client:`（父类缺省实现也不分发），所以自己分：
        /// Option+数字上屏候选的译文（Option 会把数字键变成 ¡™£ 这类字符，只能按键码认）；命令键按键码映射成原来的选择器；
        /// 其余按事件带的字符走文本路径。返回 true 表示已处理，系统不再把按键交给应用。
        // define_class! 会把返回类型转成 ObjC BOOL，方法体里不能用 `return`，逻辑放在下面的 inherent impl
        #[unsafe(method(handleEvent:client:))]
        fn handle_event(&self, event: Option<&NSEvent>, client: Option<&AnyObject>) -> bool {
            match (event, client) {
                (Some(event), Some(client)) => {
                    let client = TextClient::new(client);
                    // panic 拦下后把缓冲区原样上屏，这个按键交还给应用
                    catch_panic("handleEvent", || self.dispatch_event(event, client))
                        .unwrap_or_else(|| {
                            recover_from_panic(Some(client));
                            false
                        })
                }
                _ => false,
            }
        }

        /// 单击 Shift 的短暂等待期结束：若没有迟到的组合键 KeyDown，就确认切换。
        #[unsafe(method(confirmPendingSwitch:))]
        fn confirm_pending_switch(&self, timer: Option<&NSTimer>) {
            let done = catch_panic("confirmPendingSwitch", || {
                let Some(timer) = timer else { return };
                let Some(client) = timer.userInfo() else {
                    return;
                };
                let flipped = host::with(|h| {
                    if !h
                        .pending_switch
                        .as_ref()
                        .is_some_and(|scheduled| std::ptr::eq(&**scheduled, timer))
                    {
                        return false;
                    }
                    h.pending_switch.take();
                    h.mode.confirm_pending()
                })
                .unwrap_or(false);
                if flipped {
                    self.switch_language(TextClient::new(&client));
                }
            });
            if done.is_none() {
                recover_from_panic(None);
            }
        }

        /// 应用要求立刻结束本次输入（切换焦点、切换输入法等）。
        #[unsafe(method(commitComposition:))]
        fn commit_composition(&self, client: Option<&AnyObject>) {
            let client = client.map(TextClient::new);
            let done = catch_panic("commitComposition", || {
                if let Some(client) = client {
                    self.commit_raw(client);
                }
                host::with(|h| {
                    h.cancel_prediction();
                    h.window.hide();
                });
            });
            if done.is_none() {
                recover_from_panic(client);
            }
        }

        #[unsafe(method(activateServer:))]
        fn activate_server(&self, sender: Option<&AnyObject>) {
            tracing::info!("activateServer");
            let done = catch_panic("activateServer", || {
                // 用户要往 [apps] 里加应用时，从这条日志抄 bundle identifier
                let bundle = sender.and_then(|s| TextClient::new(s).bundle_identifier());
                if let Some(bundle) = &bundle {
                    tracing::debug!(%bundle, "当前应用");
                }
                host::with(|h| {
                    h.engine.set_application(bundle);
                    h.refresh_text_replacements();
                    h.reload_config_if_changed();
                    h.indicator.activate(h.mode.english());
                    h.watch.start();
                });
            });
            if done.is_none() {
                recover_from_panic(None);
            }
        }

        /// 系统输入源菜单（菜单栏旗帜图标）每次展开前来取输入法自己的条目。
        #[unsafe(method_id(menu))]
        fn menu(&self) -> Option<Retained<NSMenu>> {
            host::with(|h| h.menu.ns_menu())
        }

        /// 输入源菜单里点了条目：IMK 转发到控制器，sender 是带 IMKCommandMenuItem 的字典。
        #[unsafe(method(menuAction:))]
        fn menu_action(&self, sender: Option<&AnyObject>) {
            if let Some(action) = menubar::action_from_sender(sender) {
                host::with(|h| h.perform(action));
            }
        }

        #[unsafe(method(deactivateServer:))]
        fn deactivate_server(&self, sender: Option<&AnyObject>) {
            tracing::info!("deactivateServer");
            let client = sender.map(TextClient::new);
            let done = catch_panic("deactivateServer", || {
                if let Some(client) = client {
                    self.commit_raw(client);
                }
                // 切换输入源时无论如何都收掉候选框，不能留一个孤儿窗口在屏幕上
                host::with(|h| {
                    if let Some(timer) = h.pending_switch.take() {
                        timer.invalidate();
                    }
                    h.mode.cancel_pending();
                    h.cancel_prediction();
                    h.window.hide();
                    h.indicator.deactivate();
                    h.watch.stop();
                    h.engine.break_chain();
                    h.engine.flush_learning();
                    h.last_flush = std::time::Instant::now();
                });
            });
            if done.is_none() {
                recover_from_panic(client);
                // 善后里没做的收尾：学习数据还是要落盘
                host::with(|h| {
                    h.indicator.deactivate();
                    h.watch.stop();
                    h.engine.flush_learning();
                });
            }
        }
    }

    unsafe impl NSObjectProtocol for QingjianInputController {}
);

/// 翻译选中文字最多接受多少个字符：再长既慢又贵，也不是输入法该干的事。
const MAX_TRANSLATE_CHARS: usize = 500;

/// 给本地整句模型看的光标前文最多读多少字符（Engine 自己再按它的前文长度截）。
const RESCORE_LOOKBACK: usize = qingjian_core::RESCORE_CONTEXT_CHARS;

/// 登录 / 锁屏窗口的 bundle identifier。
const LOGIN_WINDOW: &str = "com.apple.loginwindow";

/// 数字行与小键盘的键码对应的数字 1–9（ANSI 布局的物理键）。
fn digit_key(key_code: u16) -> Option<usize> {
    Some(match key_code {
        18 | 83 => 1,
        19 | 84 => 2,
        20 | 85 => 3,
        21 | 86 => 4,
        23 | 87 => 5,
        22 | 88 => 6,
        26 | 89 => 7,
        28 | 91 => 8,
        25 | 92 => 9,
        _ => return None,
    })
}

impl QingjianInputController {
    /// 登录 / 锁屏窗口：输入源菜单里没有青简，loginwindow 却照样激活它，按键一律交还系统。
    ///
    /// TODO(#190): 临时防护。现象是开机登录界面打不进模式键（u / i），推断为按键进了青简的组句；
    /// 日志只证实 loginwindow 会激活青简，按键是否真的送来没有复现（开 FileVault 的机器进不到这个界面）。
    /// 找到按键送进来的条件后改成针对它的判断，并确认别的系统界面有没有同样的情况。
    fn in_login_window(&self) -> bool {
        host::with(|h| h.engine.application() == Some(LOGIN_WINDOW)).unwrap_or(false)
    }

    /// 英文 KeyDown 只作废修饰键单击并放行；中文模式才识别应用、快捷键与文本。
    fn dispatch_event(&self, event: &NSEvent, client: TextClient<'_>) -> bool {
        if event.r#type() == NSEventType::KeyDown {
            // 先用系统给这次 KeyDown 的修饰标志消解待定单击；不读取字符。
            // Shift 抬起回调可能先于组合键 KeyDown 到达，仍带 Shift 的 KeyDown 必须作废那次切换。
            let flags = event.modifierFlags();
            let flipped = host::with(|h| {
                if let Some(timer) = h.pending_switch.take() {
                    timer.invalidate();
                }
                h.mode.key_down(
                    flags.contains(NSEventModifierFlags::Shift),
                    flags.contains(NSEventModifierFlags::Control),
                )
            })
            .unwrap_or(false);
            if flipped {
                self.switch_language(client);
            }
            // 必须早于应用识别、字符读取和所有 Engine 调用。
            if host::with(|h| Route::key_down(&h.mode)).unwrap_or(Route::Passthrough)
                == Route::Passthrough
            {
                return false;
            }
        }
        self.note_application(&client);
        if self.in_login_window() {
            return false;
        }
        // Shift / Ctrl 的按下与抬起只作为 FlagsChanged 送来，单击切换中 / 英要在这里判定（见 [`super::mode`]）
        match event.r#type() {
            NSEventType::FlagsChanged => return self.handle_flags(event, client),
            NSEventType::LeftMouseDown
            | NSEventType::RightMouseDown
            | NSEventType::OtherMouseDown => {
                self.commit_raw(client);
                return false;
            }
            NSEventType::KeyDown => {}
            _ => return false,
        }
        let flags = event.modifierFlags();
        let (command, control, option, shift) = (
            flags.contains(NSEventModifierFlags::Command),
            flags.contains(NSEventModifierFlags::Control),
            flags.contains(NSEventModifierFlags::Option),
            flags.contains(NSEventModifierFlags::Shift),
        );
        let key = event.keyCode();
        let pressed = Modifiers {
            option,
            shift,
            control,
            command,
        };
        // 提示在显示：敲任何键先收掉，键照常处理
        host::with(|h| h.clear_notice());
        let reviewing = host::with(|h| h.translation.is_some()).unwrap_or(false);
        let hotkey = self.translate_hotkey(event, pressed);
        match Route::decide(false, reviewing, hotkey) {
            Route::Passthrough => return false,
            // 翻译选中文字进行中：回车 / 空格 / 1 接受，Esc 放弃，其他键放弃后照常交给应用
            Route::Review => return self.handle_translation_review(key, client),
            Route::Translate => return self.translate_selection(client),
            Route::Continue => {}
        }
        // 修饰键 + 数字：按配置的两组组合上屏第一 / 第二个译词（缺省 ⌥ 与 ⇧⌥）、删候选（缺省 ⇧）。
        // 只在组句中认：不在组句时 ⇧4 就是 `$`，得走下面的标点转换（中文模式出 ￥、⇧6 出 ……、⇧1 出 ！），
        // 以前在这里被截走后原样还给应用，全角转换就没机会做了。
        // 表达式模式（`v2^3`）里 ⇧+数字打的是 `^ * ( )`，不当快捷键
        let composing = host::with(|h| !h.engine.composition().is_empty()).unwrap_or(false);
        let expression = composing && host::with(|h| h.engine.expression_mode()).unwrap_or(false);
        if composing
            && !expression
            && !pressed.is_empty()
            && let Some(digit) = digit_key(key)
        {
            let (first, second) = host::with(|h| h.translation_keys).unwrap_or_default();
            if pressed == first {
                return self.handle_translation_key(digit, 0, client);
            }
            if pressed == second {
                return self.handle_translation_key(digit, 1, client);
            }
            if pressed == host::with(|h| h.delete_keys).unwrap_or_default() {
                return self.handle_delete_key(digit, client);
            }
        }
        let selector = match key {
            36 | 76 => Some(sel!(insertNewline:)),
            48 if shift => Some(sel!(insertBacktab:)),
            48 => Some(sel!(insertTab:)),
            51 if option => Some(sel!(deleteWordBackward:)),
            51 if command => Some(sel!(deleteToBeginningOfLine:)),
            51 => Some(sel!(deleteBackward:)),
            117 => Some(sel!(deleteForward:)),
            53 => Some(sel!(cancelOperation:)),
            126 => Some(sel!(moveUp:)),
            125 => Some(sel!(moveDown:)),
            123 if command => Some(sel!(moveToLeftEndOfLine:)),
            124 if command => Some(sel!(moveToRightEndOfLine:)),
            123 if option => Some(sel!(moveWordLeft:)),
            124 if option => Some(sel!(moveWordRight:)),
            123 => Some(sel!(moveLeft:)),
            124 => Some(sel!(moveRight:)),
            116 => Some(sel!(pageUp:)),
            121 => Some(sel!(pageDown:)),
            115 => Some(sel!(moveToBeginningOfLine:)),
            119 => Some(sel!(moveToEndOfLine:)),
            _ => None,
        };
        if let Some(selector) = selector {
            return self.handle_command(selector, client);
        }
        if command || control {
            return false;
        }
        match event.characters() {
            Some(text) if !text.is_empty() => self.handle_text(&text.to_string(), client, pressed),
            _ => false,
        }
    }

    /// 这一键是不是翻译选中文字的快捷键（配置 `[shortcut] translate_selection`，缺省 `⌃⌥T`）。
    /// 组句中不算：那段输入归青简，先打完再说。
    fn translate_hotkey(&self, event: &NSEvent, pressed: Modifiers) -> bool {
        let typed = event
            .charactersIgnoringModifiers()
            .map(|c| c.to_string().to_ascii_lowercase());
        let combo = host::with(|h| h.translate_keys).unwrap_or_default();
        pressed == combo.modifiers
            && typed.as_deref().and_then(|t| t.chars().next()) == Some(combo.key)
            && !host::with(|h| !h.engine.composition().is_empty()).unwrap_or(false)
    }
}
