//! 联合解码热路径计时与规模统计：新 Engine 首次查询（一次性建共现索引）、
//! 同一 Engine 第二次查询（稳态）、逐键冷 / 热缓存，以及每条输入的搜索规模。
//!
//! `cargo run -p qingjian-cli --release --example joint_bench -- --dict assets/lexicon/dict.tsv --lm data/generated/lm.qj`

use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use qingjian_core::Engine;
use qingjian_core::sentence::LanguageModel;
use qingjian_dictionary::Dictionary;
use qingjian_lm::BigramModel;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/generated/dict.qj")]
    dict: PathBuf,

    #[arg(long, default_value = "data/generated/lm.qj")]
    lm: PathBuf,

    /// 每个输入重复几轮。
    #[arg(long, default_value_t = 12)]
    repetitions: usize,

    #[arg(long, default_value = "")]
    context: String,

    inputs: Vec<String>,
}

const DEFAULT_INPUTS: [&str; 6] = [
    "nihao",
    "keneng",
    "fangan",
    "wojintianxiangyaoquxuexiao",
    "wan'dou'bu'xiang'wan",
    "ye'lang",
];

/// 触发一次性开销用的陪跑输入：两个音节，保证共现索引会被建起来。
const PRIMER: &str = "shide";

struct Samples(Vec<f64>);

impl Samples {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn push(&mut self, micros: f64) {
        self.0.push(micros);
    }

    fn median(&self) -> f64 {
        if self.0.is_empty() {
            return 0.0;
        }
        let mut sorted = self.0.clone();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() / 2]
    }

    fn report(&self, label: &str) {
        if self.0.is_empty() {
            return;
        }
        let mut sorted = self.0.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
        let max = sorted[sorted.len() - 1];
        println!(
            "  {label:<14} n={:<4} 中位数 {median:8.1}µs  P95 {p95:8.1}µs  最大 {max:8.1}µs",
            self.0.len()
        );
    }
}

fn build(args: &Args) -> Result<Engine, Box<dyn std::error::Error>> {
    let dictionary = Dictionary::from_path(&args.dict)?;
    let mut engine = Engine::new(dictionary);
    if args.lm.is_file() {
        let model: Box<dyn LanguageModel> = Box::new(BigramModel::from_path(&args.lm)?);
        engine = engine.with_language_model(model);
    }
    Ok(engine)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let inputs: Vec<String> = if args.inputs.is_empty() {
        DEFAULT_INPUTS
            .iter()
            .map(|input| (*input).to_owned())
            .collect()
    } else {
        args.inputs.clone()
    };
    let mut first = Samples::new();
    let mut steady = Samples::new();
    let mut cold = Samples::new();
    let mut typed = Samples::new();
    println!("规模（每条输入第一轮）");
    for input in &inputs {
        let mut per_input = Samples::new();
        for round in 0..args.repetitions {
            let mut engine = build(&args)?;
            engine.set_rescoring_context(Some(args.context.clone()));
            if round == 0 {
                engine.set_input(input);
                let started = Instant::now();
                let query = engine.query()?;
                first.push(started.elapsed().as_secs_f64() * 1e6);
                let stats = engine.last_joint_stats();
                println!(
                    "{input}\n  候选 {} 条，首选 {}；parser {} / 有资格 {} / 探针 {} / 搜索 {}，词格 {}，路径 {}，组合 {}",
                    query.candidates.items.len(),
                    query.candidates.items.first().map_or("-", |c| &c.text),
                    stats.parser_segmentations,
                    stats.eligible_segmentations,
                    stats.probed_segmentations,
                    stats.searched_segmentations,
                    stats.spans,
                    stats.viterbi_paths,
                    stats.generated_candidates,
                );
            }
            // 用陪跑输入把一次性开销付掉，再量这个输入本身
            engine.set_input(PRIMER);
            let _ = engine.query()?;
            engine.set_input(input);
            let started = Instant::now();
            let _ = engine.query()?;
            let elapsed = started.elapsed().as_secs_f64() * 1e6;
            steady.push(elapsed);
            per_input.push(elapsed);
        }
        println!("  稳态中位数 {:.1}µs", per_input.median());
    }
    // 逐键：同一个 Engine 上依次喂每个前缀；先整段跑一遍热身，再分别测冷 / 热。
    let mut engine = build(&args)?;
    for input in &inputs {
        let prefixes: Vec<String> = (1..=input.chars().count())
            .map(|length| input.chars().take(length).collect())
            .filter(|prefix: &String| prefix.chars().all(|c| c.is_ascii_lowercase()))
            .collect();
        for prefix in &prefixes {
            engine.set_input(prefix);
            let _ = engine.query()?;
        }
        for prefix in &prefixes {
            engine.set_input(prefix);
            let started = Instant::now();
            let _ = engine.query()?;
            cold.push(started.elapsed().as_secs_f64() * 1e6);
        }
        for _ in 0..args.repetitions {
            for prefix in &prefixes {
                engine.set_input(prefix);
                let started = Instant::now();
                let _ = engine.query()?;
                typed.push(started.elapsed().as_secs_f64() * 1e6);
            }
        }
    }
    println!("\n耗时（微秒，Linux 本机 release，不含候选释义）");
    first.report("首次查询");
    steady.report("整串稳态");
    cold.report("逐键冷缓存");
    typed.report("逐键热缓存");
    Ok(())
}
