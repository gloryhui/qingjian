//! IMK 事件入口契约：系统必须把修饰键事件发到控制器，单击状态机才有机会执行。

use objc2::{ClassType, sel};
use objc2_app_kit::NSEventMask;

use super::{INPUT_EVENTS, QingjianInputController};

#[test]
fn controller_declares_its_own_recognized_events() {
    let selector = sel!(recognizedEvents:);
    assert!(
        QingjianInputController::class()
            .instance_methods()
            .iter()
            .any(|method| method.name() == selector),
        "控制器必须覆盖 recognizedEvents:，继承的默认实现只订阅 KeyDown，收不到 Shift 事件"
    );
}

#[test]
fn recognized_events_include_modifier_changes_and_mouse_commit() {
    assert!(INPUT_EVENTS.contains(NSEventMask::KeyDown | NSEventMask::FlagsChanged));
    assert!(INPUT_EVENTS.contains(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown
    ));
    assert!(!INPUT_EVENTS.contains(NSEventMask::KeyUp));
}
