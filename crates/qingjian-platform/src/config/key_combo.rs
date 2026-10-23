//! 可配置的修饰键与主键组合。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::modifiers::Modifiers;

/// 修饰键 + 一个字母、数字或 Enter 键的组合，配置里写成 `control+option+t`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyCombo {
    /// 修饰键，至少一个。
    pub modifiers: Modifiers,

    /// 字母或数字键（小写），Enter 用 `\r` 表示。
    pub key: char,
}

impl KeyCombo {
    pub const TRANSLATE_DEFAULT: Self = Self {
        modifiers: Modifiers {
            option: true,
            shift: false,
            control: true,
            command: false,
        },
        key: 't',
    };

    pub const COMMIT_TRANSLATION_DEFAULT: Self = Self {
        modifiers: Modifiers::OPTION,
        key: '\r',
    };

    /// 配置文件里的写法。
    pub fn key_string(&self) -> String {
        format!(
            "{}+{}",
            self.modifiers.key(),
            if self.key == '\r' {
                "enter".to_owned()
            } else {
                self.key.to_string()
            }
        )
    }

    /// 给人看的写法：`⌃⌥T`。
    pub fn label(&self) -> String {
        format!(
            "{}{}",
            self.modifiers.label(),
            if self.key == '\r' {
                "↵".to_owned()
            } else {
                self.key.to_ascii_uppercase().to_string()
            }
        )
    }
}

impl Default for KeyCombo {
    fn default() -> Self {
        Self::TRANSLATE_DEFAULT
    }
}

impl FromStr for KeyCombo {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (modifiers, key) = text
            .trim()
            .rsplit_once('+')
            .ok_or_else(|| format!("expected modifiers+key, got {text:?}"))?;
        if key.trim().eq_ignore_ascii_case("enter") || key.trim().eq_ignore_ascii_case("return") {
            return Ok(Self {
                modifiers: modifiers.parse()?,
                key: '\r',
            });
        }
        let mut chars = key.trim().chars();
        let (Some(key), None) = (chars.next(), chars.next()) else {
            return Err(format!("key must be a single character: {key:?}"));
        };
        if !key.is_ascii_alphanumeric() {
            return Err(format!("key must be a letter or digit: {key:?}"));
        }
        Ok(Self {
            modifiers: modifiers.parse()?,
            key: key.to_ascii_lowercase(),
        })
    }
}

impl TryFrom<String> for KeyCombo {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<KeyCombo> for String {
    fn from(combo: KeyCombo) -> Self {
        combo.key_string()
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_shortcuts_round_trip_and_remain_configurable() {
        let default: KeyCombo = "option+enter".parse().unwrap();
        assert_eq!(default, KeyCombo::COMMIT_TRANSLATION_DEFAULT);
        assert_eq!(default.label(), "⌥↵");
        assert_eq!(default.key_string(), "option+enter");
        assert_eq!("alt+return".parse::<KeyCombo>().unwrap(), default);
        assert_eq!(
            "control+enter".parse::<KeyCombo>().unwrap().key_string(),
            "control+enter"
        );
    }

    #[test]
    fn parses_and_labels() {
        let combo: KeyCombo = "control+option+t".parse().unwrap();
        assert_eq!(combo, KeyCombo::TRANSLATE_DEFAULT);
        assert_eq!(combo.label(), "⌃⌥T");
        assert_eq!(combo.key_string(), "control+option+t");
        assert!("t".parse::<KeyCombo>().is_err());
        assert!("option+tt".parse::<KeyCombo>().is_err());
        assert!("option+-".parse::<KeyCombo>().is_err());
        for option in ["control+shift+t", "control+option+e", "shift+command+9"] {
            assert_eq!(option.parse::<KeyCombo>().unwrap().key_string(), option);
        }
    }
}
