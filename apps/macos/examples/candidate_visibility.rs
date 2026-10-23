//! 主线程窗口冒烟：cargo run -p qingjian-macos --example candidate_visibility --locked。
//! 显式运行时短暂显示独立候选面板，不注册输入源，也不读取用户配置或学习数据。
//! 完整隐藏回归需将二进制放入 LSBackgroundOnly 的 app 再启动，步骤见 crate-notes。

#[allow(dead_code, unused_imports)]
#[path = "../src/candidates/mod.rs"]
mod candidates;

use candidates::{CandidateWindow, Frame, Preedit, Row};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace};
use objc2_foundation::{NSPoint, NSRect, NSSize};

fn main() {
    let result = std::panic::catch_unwind(check_window);
    if let Some(path) = std::env::var_os("QINGJIAN_WINDOW_REPORT") {
        let report = match &result {
            Ok(()) => "通过：三轮隐藏恢复，前台焦点未改变".to_owned(),
            Err(error) => format!(
                "失败：{}",
                error
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| error.downcast_ref::<&str>().copied())
                    .unwrap_or("未知错误")
            ),
        };
        std::fs::write(path, report).expect("无法保存窗口验证结果");
    }
    if result.is_err() {
        std::process::exit(1);
    }
}

fn check_window() {
    let mtm = MainThreadMarker::new().expect("窗口回归必须在主线程运行");
    let app = NSApplication::sharedApplication(mtm);
    assert!(app.setActivationPolicy(NSApplicationActivationPolicy::Accessory));
    app.finishLaunching();
    let workspace = NSWorkspace::sharedWorkspace();
    let front = workspace
        .frontmostApplication()
        .map(|a| a.processIdentifier());
    let mut window = CandidateWindow::new(mtm);
    let frame = Frame {
        preedit: Preedit::plain("ni'hao", 6),
        rows: vec![Row {
            index: "1".to_owned(),
            text: "你好".to_owned(),
            annotation: Vec::new(),
            cloud: false,
        }],
        ..Frame::default()
    };
    let anchor = NSRect::new(NSPoint::new(300.0, 500.0), NSSize::new(1.0, 16.0));
    for round in 1..=3 {
        window.show(frame.clone(), anchor, None);
        app.hide(None);
        assert!(app.isHidden(), "第 {round} 轮未能构造应用隐藏状态");
        assert!(app.windows().iter().all(|w| !w.isVisible()));
        window.show(frame.clone(), anchor, None);
        assert!(!app.isHidden(), "第 {round} 轮输入法仍处于应用隐藏状态");
        assert!(app.windows().iter().any(|w| w.isVisible()));
        assert_eq!(
            workspace
                .frontmostApplication()
                .map(|a| a.processIdentifier()),
            front,
            "候选恢复不得抢占前台应用"
        );
        window.hide();
        println!("第 {round} 轮通过：隐藏状态已构造，候选已恢复，前台焦点未改变");
    }
}
