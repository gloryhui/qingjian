//! 配置解析、默认值与迁移回归测试。

use super::*;

#[test]
fn template_parses_to_defaults() {
    let config: Config = toml::from_str(TEMPLATE).unwrap();
    let mut expected = Config::default();
    // macOS 的模板一上来就把英文候选写成关（纯直通是 macOS 的缺省体验），Rust 缺省仍是开：
    // 老配置没写这一项时行为不变。见 template_english_candidates!
    expected.general.english_candidates = !cfg!(target_os = "macos");
    assert_eq!(config, expected);
}

#[test]
fn partial_file_keeps_other_defaults() {
    let config: Config = toml::from_str("[predict]\nenabled = true\nlookback = 10\n").unwrap();
    assert!(config.predict.enabled);
    assert_eq!(config.predict.lookback, 10);
    assert_eq!(config.predict.model, "deepseek-v4-flash");
    assert_eq!(config.predict.reasoning_effort, "none");
    assert_eq!(config.predict.api_key_env, "QINGJIAN_API_KEY");
}

#[test]
fn emoji_candidates_default_off_and_parse_explicit_on() {
    assert!(!Config::default().general.emoji_candidates);
    let old: Config = toml::from_str("[general]\npage_size = 5\n").unwrap();
    assert!(!old.general.emoji_candidates);
    let enabled: Config = toml::from_str("[general]\nemoji_candidates = true\n").unwrap();
    assert!(enabled.general.emoji_candidates);
    assert!(TEMPLATE.contains("emoji_candidates = false"));
}

#[test]
fn fuzzy_section_parses() {
    let config: Config = toml::from_str("[fuzzy]\nz_zh = true\nan_ang = true\n").unwrap();
    assert!(config.fuzzy.z_zh && config.fuzzy.an_ang && !config.fuzzy.n_l);
    assert!(config.fuzzy.any());
}

#[test]
fn general_and_shortcut_sections_parse() {
    let config: Config = toml::from_str(
        "[general]\npage_size = 5\npage_keys = \"[]\"\ntheme = \"dark\"\nlayout = \"horizontal\"\npreedit = \"window\"\n[shortcut]\nexpression = \"i\"\n",
    )
    .unwrap();
    assert_eq!(config.general.page_size(), 5);
    assert_eq!(config.general.page_keys(), ('[', ']'));
    assert_eq!(config.general.theme, ThemeMode::Dark);
    assert_eq!(config.general.layout, LayoutMode::Horizontal);
    assert_eq!(config.general.preedit, PreeditMode::Window);
    assert_eq!(config.general.learning_language, "en");
    assert!(config.general.english_candidates);
    assert!(!config.general.traditional);
    assert_eq!(config.general.shuangpin(), None);
    assert_eq!(config.general.log_level, LogLevel::Info);
    assert_eq!(config.shortcut.mode.expression, 'i');
    assert_eq!(config.shortcut.mode.question, 'u');
    // 译词修饰键缺省分平台（Windows 是 Ctrl 系，其余 Option 系，见 shortcut.rs），断言跟着 Default 走
    assert_eq!(
        config.shortcut.translation,
        Config::default().shortcut.translation
    );
    assert_eq!(config.shortcut.switch_mode, SwitchKeys::default());
    assert!(config.general.english_mode);
}

#[test]
fn set_value_writes_strings_and_integers() {
    let path = std::env::temp_dir().join("qingjian-config-set-value-test.toml");
    let _ = std::fs::remove_file(&path);
    Config::set_value(&path, "general", "page_size", 5i64).unwrap();
    Config::set_value(&path, "general", "theme", "dark").unwrap();
    Config::set_value(&path, "shortcut", "question", "i").unwrap();
    let config = Config::load(&path).unwrap();
    assert_eq!(config.general.page_size, 5);
    assert_eq!(config.general.theme, ThemeMode::Dark);
    assert_eq!(config.shortcut.mode.question, 'i');
    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_bool_keeps_comments_and_flips_only_that_key() {
    let path = std::env::temp_dir().join("qingjian-config-set-bool-test.toml");
    std::fs::write(
        &path,
        "# 头注释\n[fuzzy]\n# 说明\nz_zh = false\nn_l = true\n",
    )
    .unwrap();
    Config::set_bool(&path, "fuzzy", "z_zh", true).unwrap();
    Config::set_bool(&path, "predict", "enabled", true).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("# 头注释\n[fuzzy]\n# 说明\nz_zh = true\nn_l = true\n"),
        "{text}"
    );
    let config = Config::load(&path).unwrap();
    assert!(config.fuzzy.z_zh && config.fuzzy.n_l && config.predict.enabled);
    let _ = std::fs::remove_file(&path);
}

/// 老 macOS 配置升级后第一次启动就变成纯直通：`english_candidates = true` 改成 `false`，
/// 注释与其余内容原样留着；印记文件保证用户后来自己打开的 `true` 不再被动。
#[test]
fn pure_english_migration_flips_old_configs_once() {
    let dir = std::env::temp_dir().join("qingjian-pure-english-migrate-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[general]\n# 英文模式也给候选\nenglish_candidates = true\npage_size = 9\n",
    )
    .unwrap();
    assert!(Config::migrate_macos_pure_english(&path).unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# 英文模式也给候选\nenglish_candidates = false\npage_size = 9\n"),
        "{text}"
    );
    assert!(!Config::load(&path).unwrap().general.english_candidates);

    Config::set_bool(&path, "general", "english_candidates", true).unwrap();
    assert!(!Config::migrate_macos_pure_english(&path).unwrap());
    assert!(Config::load(&path).unwrap().general.english_candidates);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 没写这一项的老配置补上 `false`（缺省是开，纯直过得写下来）；已经是 `false` 的连文件都不碰。
#[test]
fn pure_english_migration_adds_a_missing_key_and_skips_a_pure_config() {
    let dir = std::env::temp_dir().join("qingjian-pure-english-add-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[general]\npage_size = 9\n").unwrap();
    assert!(Config::migrate_macos_pure_english(&path).unwrap());
    assert!(!Config::load(&path).unwrap().general.english_candidates);

    std::fs::write(&path, "[general]\nenglish_candidates = false\n").unwrap();
    let _ = std::fs::remove_file(path.with_file_name(PURE_ENGLISH_STAMP));
    assert!(!Config::migrate_macos_pure_english(&path).unwrap());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[general]\nenglish_candidates = false\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 本来就是 `false` 的配置（新装读到的模板）也要留下印记：印记记的是「这次检查做过了」，
/// 避免下一次启动重复检查；macOS 英文直通不依赖这个历史设置的当前值。
#[test]
fn pure_english_migration_stamps_a_config_that_was_already_pure() {
    let dir = std::env::temp_dir().join("qingjian-pure-english-stamp-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[general]\nenglish_candidates = false\n").unwrap();

    assert!(!Config::migrate_macos_pure_english(&path).unwrap());
    assert!(path.with_file_name(PURE_ENGLISH_STAMP).exists());

    Config::set_bool(&path, "general", "english_candidates", true).unwrap();
    assert!(!Config::migrate_macos_pure_english(&path).unwrap());
    assert!(Config::load(&path).unwrap().general.english_candidates);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 配置写不进去时不能提前留下印记：印记等于「迁移做过了」，先落下就会让坏在半路的配置永远不再被修。
/// 搬开阻塞再跑一次，迁移照旧完成。
#[test]
fn pure_english_migration_leaves_no_stamp_when_the_config_write_fails() {
    let dir = std::env::temp_dir().join("qingjian-pure-english-write-fail-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[general]\nenglish_candidates = true\n").unwrap();
    // 原子写先写同目录的 `.<配置文件名>.tmp-<进程号>`：拿一个目录占住它，创建就失败。
    let blocker = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    std::fs::create_dir_all(&blocker).unwrap();

    assert!(matches!(
        Config::migrate_macos_pure_english(&path),
        Err(ConfigError::Write { .. })
    ));
    assert!(Config::load(&path).unwrap().general.english_candidates);
    assert!(!path.with_file_name(PURE_ENGLISH_STAMP).exists());

    std::fs::remove_dir(&blocker).unwrap();
    assert!(Config::migrate_macos_pure_english(&path).unwrap());
    assert!(!Config::load(&path).unwrap().general.english_candidates);
    assert!(path.with_file_name(PURE_ENGLISH_STAMP).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 配置文件读不了（语法坏了）时既不碰配置也不落印记：修好之后升级仍然该迁这一次。
#[test]
fn pure_english_migration_refuses_a_broken_config() {
    let dir = std::env::temp_dir().join("qingjian-pure-english-broken-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[general\nenglish_candidates = true\n").unwrap();
    assert!(matches!(
        Config::migrate_macos_pure_english(&path),
        Err(ConfigError::Edit { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[general\nenglish_candidates = true\n"
    );
    assert!(!path.with_file_name(PURE_ENGLISH_STAMP).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writes_create_the_data_directory_for_a_fresh_account() {
    let dir = std::env::temp_dir().join("qingjian-config-fresh-account-test");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("Qingjian").join("config.toml");
    assert!(Config::write_template_if_missing(&path).unwrap());
    assert!(!Config::write_template_if_missing(&path).unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
    // 没有模板直接保存也行
    std::fs::remove_dir_all(&dir).unwrap();
    Config::set_bool(&path, "predict", "enabled", true).unwrap();
    assert!(Config::load(&path).unwrap().predict.enabled);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn set_bool_starts_from_template_when_missing() {
    let path = std::env::temp_dir().join("qingjian-config-set-bool-missing-test.toml");
    let _ = std::fs::remove_file(&path);
    Config::set_bool(&path, "predict", "enabled", true).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# 青简输入法配置"));
    assert!(Config::load(&path).unwrap().predict.enabled);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_bool_refuses_broken_file() {
    let path = std::env::temp_dir().join("qingjian-config-set-bool-broken-test.toml");
    std::fs::write(&path, "[fuzzy\nz_zh = false\n").unwrap();
    assert!(matches!(
        Config::set_bool(&path, "fuzzy", "z_zh", true),
        Err(ConfigError::Edit { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[fuzzy\nz_zh = false\n"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn missing_file_is_default() {
    let path = std::env::temp_dir().join("qingjian-config-missing-test.toml");
    let _ = std::fs::remove_file(&path);
    assert_eq!(Config::load(&path).unwrap(), Config::default());
}
