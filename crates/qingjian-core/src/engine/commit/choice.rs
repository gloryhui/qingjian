//! 输入串与读音共享的选择记录，供撤销时逐项退回。

/// 一次「输入串 → 文本」的选择记录：撤销时按它退回 [`crate::Learner::record_choice`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// 候选的读音键（音节无分隔符连起来）；形码候选没有读音是空串。
    pub reading: String,

    /// 记录的文本。
    pub text: String,
}
