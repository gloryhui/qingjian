use std::time::Duration;

/// 一次候选查询各阶段的耗时。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timings {
    /// 拼音切分。
    pub parse: Duration,

    /// 词库查询（含所有切分）。
    pub lookup: Duration,

    /// 排序与去重。
    pub rank: Duration,
}

impl Timings {
    pub fn total(&self) -> Duration {
        self.parse + self.lookup + self.rank
    }
}

/// 最近一次查询里联合整句搜索的规模。只给评测工具与测试读，不参与任何排序。
///
/// 「探针」指每个候选切分先跑一次单前驱、不带路线串的窄搜索，用它的最优路径分
/// 决定哪几条切分值得做完整的多样化搜索（见 `engine::query::joint`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JointStats {
    /// parser 给出的切分数。
    pub parser_segmentations: usize,

    /// 通过完整性过滤、有资格参与联合搜索的切分数。
    pub eligible_segmentations: usize,

    /// 跑了探针的切分数。
    pub probed_segmentations: usize,

    /// 真正做了多路径搜索的切分数。
    pub searched_segmentations: usize,

    /// 本次查询新建的词格数（跨度 / lattice）。
    pub spans: usize,

    /// 联合搜索生成、参与排序的整句路径数。
    pub viterbi_paths: usize,

    /// 提取出的未登录组合候选数。
    pub generated_candidates: usize,
}
