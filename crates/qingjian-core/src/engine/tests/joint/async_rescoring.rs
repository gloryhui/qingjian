//! 后台重排的请求、前文、门槛与回查回归。

use super::ranking::WideGapBigram;
use super::*;
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct ControlledScorer {
    submitted: mpsc::Sender<(String, Vec<String>)>,
    release: mpsc::Receiver<()>,
    target: &'static str,
}

impl SentenceScorer for ControlledScorer {
    fn score(&self, context: &str, texts: &[&str]) -> Vec<f64> {
        self.submitted
            .send((
                context.to_owned(),
                texts.iter().map(|s| (*s).to_owned()).collect(),
            ))
            .unwrap();
        self.release.recv().unwrap();
        texts
            .iter()
            .map(|text| if *text == self.target { -1.0 } else { -20.0 })
            .collect()
    }
}

#[test]
fn joint_async_query_request_poll_query_preserves_context_and_preedit() {
    let dictionary = Dictionary::parse(
        "晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let (submitted_tx, submitted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(BiasedBigram))
        .with_async_sentence_scorer(
            Box::new(ControlledScorer {
                submitted: submitted_tx,
                release: release_rx,
                target: "玩都不想玩",
            }),
            Some(1.0),
            None,
            Some(4),
        );
    engine.set_rescoring_context(Some("应用里的上文".to_owned()));
    engine.set_input("wan'dou'bu'xiang'wan");
    let first = engine.query().unwrap();
    assert_eq!(first.candidates.items[0].text, "晚都不想玩");
    assert!(engine.request_rescoring());
    let (context, texts) = submitted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(context, "里的上文");
    assert!(texts.iter().any(|text| text == "玩都不想玩"));
    assert!(!engine.poll_rescoring());
    release_tx.send(()).unwrap();
    let started = Instant::now();
    while !engine.poll_rescoring() {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    let rescored = engine.query().unwrap();
    assert_eq!(rescored.candidates.items[0].text, "玩都不想玩");
    assert_eq!(rescored.marked_text(), "wan'dou'bu'xiang'wan");
    engine.set_rescoring_context(Some("换了前文".to_owned()));
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "晚都不想玩"
    );
    assert!(engine.request_rescoring());
    let (context, _) = submitted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(context, "换了前文");
    release_tx.send(()).unwrap();
    let started = Instant::now();
    while !engine.poll_rescoring() {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "玩都不想玩"
    );
}

#[test]
fn async_margin_never_submits_an_ineligible_path() {
    let dictionary = Dictionary::parse(
        "晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let (submitted_tx, submitted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(WideGapBigram))
        .with_async_sentence_scorer(
            Box::new(ControlledScorer {
                submitted: submitted_tx,
                release: release_rx,
                target: "玩都不想玩",
            }),
            Some(1.0),
            Some(4.0),
            None,
        );
    engine.set_input("wan'dou'bu'xiang'wan");
    assert_ne!(
        engine.query().unwrap().candidates.items[0].text,
        "玩都不想玩"
    );
    assert!(engine.request_rescoring());
    let (_, texts) = submitted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!texts.iter().any(|text| text == "玩都不想玩"));
    release_tx.send(()).unwrap();
    let started = Instant::now();
    while !engine.poll_rescoring() {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert_ne!(
        engine.query().unwrap().candidates.items[0].text,
        "玩都不想玩"
    );
}

#[test]
fn in_flight_score_from_old_context_is_discarded() {
    let dictionary = Dictionary::parse(
        "晚\twan\t9000\n万\twan\t8000\n玩\twan\t7000\n都\tdou\t6000\n不想\tbu xiang\t5000\n",
    )
    .unwrap();
    let (submitted_tx, submitted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut engine = Engine::new(dictionary)
        .with_language_model(Box::new(BiasedBigram))
        .with_async_sentence_scorer(
            Box::new(ControlledScorer {
                submitted: submitted_tx,
                release: release_rx,
                target: "玩都不想玩",
            }),
            Some(1.0),
            None,
            Some(4),
        );
    engine.set_input("wan'dou'bu'xiang'wan");
    engine.set_rescoring_context(Some("旧前文".to_owned()));
    engine.query().unwrap();
    assert!(engine.request_rescoring());
    assert_eq!(
        submitted_rx.recv_timeout(Duration::from_secs(2)).unwrap().0,
        "旧前文"
    );
    engine.set_rescoring_context(Some("新前文".to_owned()));
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "晚都不想玩"
    );
    release_tx.send(()).unwrap();
    assert!(engine.request_rescoring());
    assert_eq!(
        submitted_rx.recv_timeout(Duration::from_secs(2)).unwrap().0,
        "新前文"
    );
    assert!(!engine.poll_rescoring());
    release_tx.send(()).unwrap();
    let started = Instant::now();
    while !engine.poll_rescoring() {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "玩都不想玩"
    );
}

#[test]
fn async_neural_score_does_not_override_learned_complete_word() {
    let dictionary = Dictionary::parse("方案\tfang an\t1000\n反感\tfan gan\t100000\n").unwrap();
    let mut counts = HashMap::new();
    counts.insert("方案".to_owned(), 2);
    counts.insert("fangan\t方案".to_owned(), 2);
    let mut engine = Engine::new(dictionary)
        .with_learner(Box::new(CountingLearner(counts)))
        .with_async_sentence_scorer(
            Box::new(PrefersWholeSentence {
                target: "反感",
                seen: Arc::new(Mutex::new(Vec::new())),
            }),
            Some(1.0),
            None,
            None,
        );
    engine.set_input("fangan");
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "方案");
    assert!(engine.request_rescoring());
    let started = Instant::now();
    while !engine.poll_rescoring() {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "方案");
    assert_eq!(query.marked_text(), "fang'an");
}
