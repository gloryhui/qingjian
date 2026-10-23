//! 词库查询命中与单字读音占比。

/// 一次查询命中的词目，全部借用自词库。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match<'a> {
    /// 词。
    pub text: &'a str,

    /// 空格分隔的音节。
    pub pinyin: &'a str,

    /// 静态词频。
    pub frequency: u32,

    /// 单字所有读音的总词频；不是多音字时为 0。
    pub reading_total: u32,

    /// 音节数与查询模式的长度一致（而不是以查询为前缀的更长词）。
    pub exact: bool,
}

impl<'a> Match<'a> {
    pub fn syllables(&self) -> impl Iterator<Item = &'a str> {
        self.pinyin.split(' ')
    }

    /// 这个读音占该字总词频的比例，非多音字为 1。
    pub fn reading_share(&self) -> f64 {
        if self.reading_total == 0 {
            return 1.0;
        }
        ((f64::from(self.frequency) + 1.0) / (f64::from(self.reading_total) + 1.0)).min(1.0)
    }

    pub fn syllable_count(&self) -> usize {
        self.pinyin.split(' ').count()
    }
}
