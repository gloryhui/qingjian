//! 按键路由回归：英文模式只直通，中文模式保留原有处理。

use super::Route;
use crate::host::ModeState;
use qingjian_core::Engine;
use qingjian_dictionary::Dictionary;
use qingjian_platform::key_tap::ModifierEvent;
use qingjian_platform::{AppsConfig, Config, SwitchKey};

fn mode(config: &Config) -> ModeState {
    let mut mode = ModeState::default();
    mode.set_settings(config.shortcut.switch_mode, config.general.english_mode);
    mode
}

fn shift(mode: &mut ModeState, bare: bool) -> bool {
    mode.modifier_event(ModifierEvent {
        switch: Some(SwitchKey::Shift),
        slot: 0,
        bare,
    })
}

fn tap_shift(mode: &mut ModeState) {
    assert!(!shift(mode, true));
    assert!(shift(mode, false));
    assert!(mode.confirm_pending());
}

fn engine() -> Engine {
    Engine::new(Dictionary::parse("你\tni\t1000\n").unwrap())
}

#[test]
fn shift_taps_switch_the_key_down_route_both_ways() {
    let mut mode = mode(&Config::default());
    assert_eq!(Route::key_down(&mode), Route::Continue);
    tap_shift(&mut mode);
    assert_eq!(Route::key_down(&mode), Route::Passthrough);
    tap_shift(&mut mode);
    assert_eq!(Route::key_down(&mode), Route::Continue);
}

#[test]
fn shift_combinations_interrupt_the_tap_in_both_languages() {
    // 路由不接收键码或字符；这些组合的 KeyDown 都只按当前模式判定。
    for key in ["A", ".", ",", "'", ";", "Tab", "1"] {
        for english in [false, true] {
            let mut mode = mode(&Config::default());
            if english {
                tap_shift(&mut mode);
            }
            assert!(!shift(&mut mode, true));
            assert_eq!(
                Route::key_down(&mode),
                if english {
                    Route::Passthrough
                } else {
                    Route::Continue
                },
                "Shift+{key}, english={english}"
            );
            assert!(!shift(&mut mode, false), "Shift+{key}");
            assert_eq!(mode.english(), english, "Shift+{key}");
            tap_shift(&mut mode);
            assert_eq!(mode.english(), !english);
        }
    }
}

#[test]
fn late_right_shift_chord_keeps_english_route_pure() {
    let mut mode = mode(&Config::default());
    tap_shift(&mut mode);
    for key in ["A", ".", "'", ";"] {
        assert!(!mode.modifier_event(ModifierEvent {
            switch: Some(SwitchKey::Shift),
            slot: 1,
            bare: true,
        }));
        assert!(mode.modifier_event(ModifierEvent {
            switch: Some(SwitchKey::Shift),
            slot: 1,
            bare: false,
        }));
        // 迟到的 KeyDown 仍带 Shift，必须先作废待定单击，再做路由。
        assert!(!mode.key_down(true, false), "Shift+{key}");
        assert_eq!(Route::key_down(&mode), Route::Passthrough);
        assert!(mode.english());
    }
}

#[test]
fn english_candidates_and_application_lists_cannot_enable_engine_routes() {
    for candidates in [false, true] {
        for excluded_apps in [vec![], vec!["*".to_owned()]] {
            let mut config = Config::default();
            config.general.english_candidates = candidates;
            config.apps = AppsConfig {
                english_candidates_off: excluded_apps,
            };
            let mut mode = mode(&config);
            tap_shift(&mut mode);
            // 即使 Engine 留着 composition 或旧英文候选状态，也不能将下一键送入文本处理。
            let mut engine = engine();
            engine.push('n');
            engine.push('i');
            engine.set_english_mode(candidates);
            assert_eq!(Route::key_down(&mode), Route::Passthrough);
            assert_eq!(engine.composition().text(), "ni");
            for reviewing in [false, true] {
                for hotkey in [false, true] {
                    assert_eq!(
                        Route::decide(mode.passthrough(), reviewing, hotkey),
                        Route::Passthrough
                    );
                }
            }
        }
    }
}

#[test]
fn switching_with_composition_commits_raw_once_before_passthrough() {
    let mut mode = mode(&Config::default());
    let mut engine = engine();
    engine.push('n');
    engine.push('i');
    tap_shift(&mut mode);
    // controller::mode::switch_language 保留这条收尾：commit_raw 使用 take_raw。
    assert_eq!(engine.take_raw(), "ni");
    assert!(engine.composition().is_empty());
    assert!(engine.take_raw().is_empty());
    assert_eq!(Route::key_down(&mode), Route::Passthrough);
    tap_shift(&mut mode);
    assert_eq!(Route::key_down(&mode), Route::Continue);
    engine.push('n');
    engine.push('i');
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .any(|candidate| candidate.text == "你")
    );
}

#[test]
fn chinese_mode_keeps_configured_punctuation_conversion() {
    let mode = mode(&Config::default());
    for full_width in [false, true] {
        for (typed, expected) in [
            (',', "，"),
            ('.', "。"),
            ('"', "“"),
            (':', "："),
            ('<', "《"),
            ('>', "》"),
        ] {
            let mut engine = engine();
            engine.set_full_width_punctuation(full_width);
            assert_eq!(Route::key_down(&mode), Route::Continue);
            assert_eq!(engine.punctuate(typed), full_width.then_some(expected));
        }
    }
}

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
