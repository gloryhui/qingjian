//! 按真实词库和静态 LM 追踪任意拼音的词图、前驱竞争与束剪枝。
//!
//! 用法：cargo run -p qingjian-cli --example path_trace -- <拼音>...。

use clap::Parser;
use qingjian_core::parser;
use qingjian_core::sentence::{Personal, SentenceScorer, SpanCache, convert_paths_diagnostic};
use qingjian_dictionary::Dictionary;
use qingjian_lm::BigramModel;
use std::path::Path;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    neural: Option<std::path::PathBuf>,

    #[arg(long, default_value = "")]
    context: String,

    inputs: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let dictionary = Dictionary::from_path("data/generated/dict.qj")?;
    let model = BigramModel::from_path(Path::new("data/generated/lm.qj"))?;
    let neural = args
        .neural
        .as_deref()
        .map(qingjian_neural::CharScorer::load)
        .transpose()?;
    for input in args.inputs {
        let segmentations = parser::segment(&input)?;
        println!("INPUT {input} SEGMENTATIONS {}", segmentations.len());
        for segmentation in &segmentations {
            let positions: Vec<_> = segmentation
                .patterns()
                .into_iter()
                .map(|p| vec![p])
                .collect();
            let (paths, trace) = convert_paths_diagnostic(
                &[&dictionary],
                &positions,
                false,
                8,
                &model,
                Personal::NONE,
                |_| 0,
                |_, _| 0.0,
                &mut SpanCache::default(),
            );
            println!("SEGMENTATION {segmentation} SPANS {}", trace.spans.len());
            for span in &trace.spans {
                if !span.words.is_empty() {
                    println!("  SPAN {}..{} {:?}", span.start, span.end, span.words);
                }
            }
            for transition in &trace.transitions {
                println!("  TRANSITION {transition:?}");
            }
            for path in &trace.paths {
                println!("  PATH {path:?}");
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
            println!("  TOP {paths:?}");
        }
    }
    Ok(())
}
