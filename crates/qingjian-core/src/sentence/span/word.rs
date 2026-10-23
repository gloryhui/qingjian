//! 词图缓存内的词目，分别保留敲错代价与读音代价。

/// 词图一个格子里的一个词：从词库命中里拷出来、不再借用词库的形式，能放进 [`super::SpanCache`]。
#[derive(Debug, Clone, PartialEq)]
pub struct SpanWord {
    /// 词。
    pub text: String,

    /// 词的音节。
    pub syllables: Vec<String>,

    /// 词库静态词频。
    pub frequency: u32,

    /// 命中的音节不是敲的原样（模糊音 / 敲错变体）时的代价之和，进路径得分时扣掉；原样命中是 0。
    pub penalty: f64,

    /// 多音字的少见读音代价；不表示用户敲错。
    pub reading: f64,
}
