//! 按真实词库和静态 LM 追踪任意拼音的词图、前驱竞争与束剪枝。
//!
//! 用法：cargo run -p qingjian-cli --example path_trace -- <拼音>...
//! 词库缺省读打包过的 `data/generated/dict.qj`，`--dict` 可指向任何 TSV 或 `.qj`（如 `assets/lexicon/dict.tsv`）；
//! 语言模型缺省读 `data/generated/lm.qj`，没有就退化为一元词频（`--lm` 可显式指定）。

use clap::Parser;
use qingjian_core::parser;
use qingjian_core::sentence::{
    NoLanguageModel, Personal, SentenceScorer, SpanCache, convert_paths_diagnostic,
};
use qingjian_dictionary::Dictionary;
use qingjian_lm::BigramModel;
use std::path::{Path, PathBuf};

#[derive(Parser)]
struct Args {
    /// 词库路径（TSV 或 .qj）；缺省 data/generated/dict.qj。
    #[arg(long)]
    dict: Option<PathBuf>,

    /// 语言模型路径；缺省 data/generated/lm.qj，不存在就用一元词频兜底。
    #[arg(long)]
    lm: Option<PathBuf>,

    #[arg(long)]
    neural: Option<PathBuf>,

    #[arg(long, default_value = "")]
    context: String,

    /// 每条切分保留几条路径。
    #[arg(long, default_value_t = 8)]
    paths: usize,

    inputs: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let dict_path = args
        .dict
        .clone()
        .unwrap_or_else(|| PathBuf::from("data/generated/dict.qj"));
    let dictionary = Dictionary::from_path(&dict_path)?;
    let lm_path = args
        .lm
        .clone()
        .unwrap_or_else(|| PathBuf::from("data/generated/lm.qj"));
    let model: Box<dyn qingjian_core::sentence::LanguageModel> =
        if args.lm.is_some() || lm_path.is_file() {
            Box::new(BigramModel::from_path(Path::new(&lm_path))?)
        } else {
            Box::new(NoLanguageModel)
        };
    let neural = args
        .neural
        .as_deref()
        .map(qingjian_neural::CharScorer::load)
        .transpose()?;
    for input in args.inputs {
        let segmentations = parser::segment(&input)?;
        println!("INPUT {input} SEGMENTATIONS {}", segmentations.len());
        for (rank, segmentation) in segmentations.iter().enumerate() {
            let positions: Vec<_> = segmentation
                .patterns()
                .into_iter()
                .map(|p| vec![p])
                .collect();
            let (paths, trace) = convert_paths_diagnostic(
                &[&dictionary],
                &positions,
                false,
                args.paths,
                &*model,
                Personal::NONE,
                |_| 0,
                |_, _| 0.0,
                &mut SpanCache::default(),
            );
            println!(
                "SEGMENTATION #{rank} {segmentation} SYLLABLES {} SPANS {} PATHS {}",
                segmentation.syllables.len(),
                trace.spans.len(),
                paths.len()
            );
            for paths in &trace.paths {
                println!("  PATH {paths:?}");
            }
            for (index, path) in paths.iter().enumerate() {
                println!(
                    "  TOP{index} text={} score={:.3} static={:.3} personal={:.3} penalty={:.3} words={:?}",
                    path.text,
                    path.score,
                    path.static_score,
                    path.personal_bonus,
                    path.penalty,
                    path.words
                        .iter()
                        .map(|word| word.text.as_str())
                        .collect::<Vec<_>>()
                );
            }
            if let Some(scorer) = &neural {
                let texts: Vec<&str> = paths.iter().map(|path| path.text.as_str()).collect();
                let scores = SentenceScorer::score(scorer, &args.context, &texts);
                for (path, neural_score) in paths.iter().zip(scores) {
                    let final_score = path.score + 0.5 * (neural_score - path.static_score);
                    println!(
                        "  NEURAL {} static={} neural={} final={}",
                        path.text, path.static_score, neural_score, final_score
                    );
                }
            }
        }
    }
    Ok(())
}
