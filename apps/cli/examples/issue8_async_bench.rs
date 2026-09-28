//! 使用受控异步 scorer 测量联合解码的逐键查询延迟。
//!
//! `cargo run -p qingjian-cli --example issue8_async_bench -- --dict data/generated/dict.qj --lm data/generated/lm.qj --repetitions 20`

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use clap::Parser;
use qingjian_core::Engine;
use qingjian_core::sentence::SentenceScorer;
use qingjian_dictionary::Dictionary;
use qingjian_lm::BigramModel;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/generated/dict.qj")]
    dict: PathBuf,

    #[arg(long, default_value = "data/generated/lm.qj")]
    lm: PathBuf,

    #[arg(long, default_value_t = 20)]
    repetitions: usize,
}

struct GatedScorer {
    main_thread: ThreadId,

    started: Sender<()>,

    release: Mutex<Receiver<()>>,
}

impl SentenceScorer for GatedScorer {
    fn score(&self, _context: &str, texts: &[&str]) -> Vec<f64> {
        assert_ne!(thread::current().id(), self.main_thread);
        self.started
            .send(())
            .expect("benchmark receiver remains open");
        self.release
            .lock()
            .expect("scorer gate is not poisoned")
            .recv_timeout(Duration::from_secs(10))
            .expect("benchmark releases background scoring");
        texts
            .iter()
            .map(|text| -(text.chars().count() as f64) * 0.1)
            .collect()
    }
}

struct Samples {
    whole_query: Vec<u128>,

    cold: Vec<u128>,

    hot: Vec<u128>,

    while_scorer_busy: Vec<u128>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.repetitions == 0 {
        return Err("--repetitions must be positive".into());
    }
    let cases = [
        ("short", "nihao"),
        ("long", "wojintianxiangyaoquxuexiao"),
        ("common-prefix", "wo'jin'tian'mai'dou'bu'xiang'mai"),
        ("multi-segmentation", "keneng"),
    ];
    for (name, input) in cases {
        let mut samples = Samples {
            whole_query: Vec::new(),
            cold: Vec::new(),
            hot: Vec::new(),
            while_scorer_busy: Vec::new(),
        };
        for _ in 0..args.repetitions {
            samples.whole_query.push(measure_whole_query(&args, input)?);
            measure_case(&args, input, &mut samples)?;
        }
        println!("{name} input={input:?}");
        print_stats("whole-query-cold", &samples.whole_query);
        print_stats("cold", &samples.cold);
        print_stats("hot", &samples.hot);
        print_stats("async-busy", &samples.while_scorer_busy);
    }
    Ok(())
}

fn measure_whole_query(args: &Args, input: &str) -> Result<u128, Box<dyn std::error::Error>> {
    let dictionary = Dictionary::from_path(&args.dict)?;
    let model = BigramModel::from_path(&args.lm)?;
    let (started_tx, _started_rx) = mpsc::channel();
    let (_release_tx, release_rx) = mpsc::channel();
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(model))
        .with_async_sentence_scorer(
            Box::new(GatedScorer {
                main_thread: thread::current().id(),
                started: started_tx,
                release: Mutex::new(release_rx),
            }),
            Some(0.5),
            Some(4.0),
            None,
        );
    query_time(&mut engine, input)
}

fn measure_case(
    args: &Args,
    input: &str,
    samples: &mut Samples,
) -> Result<(), Box<dyn std::error::Error>> {
    let dictionary = Dictionary::from_path(&args.dict)?;
    let model = BigramModel::from_path(&args.lm)?;
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(model))
        .with_async_sentence_scorer(
            Box::new(GatedScorer {
                main_thread: thread::current().id(),
                started: started_tx,
                release: Mutex::new(release_rx),
            }),
            Some(0.5),
            Some(4.0),
            None,
        );
    let prefixes = prefixes(input);
    let mut background_busy = false;
    for prefix in &prefixes {
        let elapsed = query_time(&mut engine, prefix)?;
        samples.cold.push(elapsed);
        if background_busy {
            samples.while_scorer_busy.push(elapsed);
        } else if engine.rescoring_pending() {
            assert!(engine.request_rescoring());
            started_rx.recv_timeout(Duration::from_secs(10))?;
            background_busy = true;
        }
    }
    if !background_busy {
        return Err(format!("{input:?} produced no async scorer request").into());
    }

    finish_scoring(&mut engine, &release_tx)?;
    while engine.request_rescoring() {
        started_rx.recv_timeout(Duration::from_secs(10))?;
        release_tx.send(())?;
        wait_for_result(&mut engine)?;
    }

    for prefix in &prefixes {
        samples.hot.push(query_time(&mut engine, prefix)?);
    }
    Ok(())
}

fn prefixes(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    for (index, character) in input.char_indices() {
        result.push(input[..index + character.len_utf8()].to_owned());
    }
    result
}

fn query_time(engine: &mut Engine, input: &str) -> Result<u128, Box<dyn std::error::Error>> {
    let started = Instant::now();
    engine.set_input(input);
    engine.query()?;
    Ok(started.elapsed().as_nanos())
}

fn finish_scoring(
    engine: &mut Engine,
    release: &Sender<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    release.send(())?;
    wait_for_result(engine)?;
    Ok(())
}

fn wait_for_result(engine: &mut Engine) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if engine.poll_rescoring() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the controlled scorer".into());
        }
        thread::yield_now();
    }
}

fn print_stats(label: &str, values: &[u128]) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let percentile_95_index = (sorted.len() * 95).div_ceil(100) - 1;
    let percentile_95 = sorted[percentile_95_index];
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2
    } else {
        sorted[sorted.len() / 2]
    };
    println!(
        "  {label}: n={} median={:.2}us p95={:.2}us max={:.2}us",
        sorted.len(),
        nanos_to_micros(median),
        nanos_to_micros(percentile_95),
        nanos_to_micros(sorted[sorted.len() - 1]),
    );
}

fn nanos_to_micros(value: u128) -> f64 {
    value as f64 / 1_000.0
}
