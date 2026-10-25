//! CoreML FP16 workflow risk scorer.
//!
//! Models are loaded through the Swift bridge in `swift/AegisANEBridge.swift` with the requested
//! `MLComputeUnits` — `.cpuAndNeuralEngine` by default. CoreML decides the final placement;
//! [`compute_plan`] reads it back from `MLComputePlan` so callers can verify where operations run.
//!
//! * `models/AegisWorkflowFP16.mlmodelc` scores one operation per prediction (online path).
//! * `models/AegisWorkflowFP16Batch.mlmodelc` carries the same weights as 1x1 convolutions over
//!   `(1, 64, 1, N)` and scores N operations per prediction (batch path). CoreML keeps the small
//!   online network on the CPU; the batch layout is large enough for it to choose the Neural Engine.

mod features;

pub use features::{
    commerce_features, workspace_features, CommerceContext, ExecutionContext, Feature, FeatureVector, Kind,
    FEATURE_COUNT, FEATURE_KINDS, FEATURE_NAMES,
};

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

extern "C" {
    fn aegis_ane_open(path: *const c_char, compute_units: i32) -> *mut c_void;
    fn aegis_ane_close(handle: *mut c_void);
    fn aegis_ane_feature_count(handle: *mut c_void) -> i32;
    fn aegis_ane_batch_size(handle: *mut c_void) -> i32;
    fn aegis_ane_score_workflow(handle: *mut c_void, features: *const f32, count: i32, out_score: *mut f32) -> i32;
    fn aegis_ane_score_batch(handle: *mut c_void, rows: *const f32, operations: i32, out_scores: *mut f32) -> i32;
    fn aegis_ane_compute_plan_json(path: *const c_char, compute_units: i32) -> *mut c_char;
    fn aegis_ane_devices_json() -> *mut c_char;
    fn aegis_ane_last_error() -> *mut c_char;
    fn aegis_ane_free_string(ptr: *mut c_char);
}

/// `MLComputeUnits` as understood by the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ComputeUnits {
    All = 0,
    CpuOnly = 1,
    CpuAndGpu = 2,
    CpuAndNeuralEngine = 3,
}

impl ComputeUnits {
    pub fn name(self) -> &'static str {
        match self {
            ComputeUnits::All => "all",
            ComputeUnits::CpuOnly => "cpuOnly",
            ComputeUnits::CpuAndGpu => "cpuAndGPU",
            ComputeUnits::CpuAndNeuralEngine => "cpuAndNeuralEngine",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AneError {
    #[error("compiled model not found at {0} (run harness/build_model.sh)")]
    ModelMissing(PathBuf),
    #[error("CoreML: {0}")]
    CoreMl(String),
    #[error("feature vector has {got} values, the model expects {expected}")]
    Shape { expected: usize, got: usize },
    #[error("bridge returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Copies and frees a malloc'd string returned by the bridge.
fn take_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the bridge returns NUL-terminated strings from strdup; we free them exactly once.
    let text = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    unsafe { aegis_ane_free_string(ptr) };
    Some(text)
}

fn last_error() -> String {
    // SAFETY: plain call into the bridge; ownership of the returned string passes to take_string.
    take_string(unsafe { aegis_ane_last_error() }).unwrap_or_else(|| "unknown CoreML error".to_string())
}

fn c_path(path: &Path) -> Result<CString, AneError> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| AneError::CoreMl("model path contains NUL".into()))
}

/// `AEGIS_ANE_MODEL` if set, otherwise `models/AegisWorkflowFP16.mlmodelc` in this repository.
pub fn default_model_path() -> PathBuf {
    std::env::var_os("AEGIS_ANE_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/AegisWorkflowFP16.mlmodelc")
    })
}

/// `AEGIS_ANE_BATCH_MODEL` if set, otherwise `models/AegisWorkflowFP16Batch.mlmodelc`.
pub fn default_batch_model_path() -> PathBuf {
    std::env::var_os("AEGIS_ANE_BATCH_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/AegisWorkflowFP16Batch.mlmodelc")
    })
}

/// A loaded CoreML model. Predictions are serialised inside the bridge, so the scorer can be
/// shared between threads.
pub struct AneScorer {
    handle: NonNull<c_void>,
    feature_count: usize,
    batch_size: usize,
    units: ComputeUnits,
    model_path: PathBuf,
}

// SAFETY: the Swift object behind `handle` guards its mutable input buffer with an NSLock and is
// reference counted; it has no thread affinity.
unsafe impl Send for AneScorer {}
unsafe impl Sync for AneScorer {}

impl AneScorer {
    pub fn open(model_path: &Path, units: ComputeUnits) -> Result<Self, AneError> {
        if !model_path.exists() {
            return Err(AneError::ModelMissing(model_path.to_path_buf()));
        }
        let path = c_path(model_path)?;
        // SAFETY: `path` is a valid C string for the duration of the call.
        let raw = unsafe { aegis_ane_open(path.as_ptr(), units as i32) };
        let handle = NonNull::new(raw).ok_or_else(|| AneError::CoreMl(last_error()))?;
        // SAFETY: `handle` was just returned by aegis_ane_open and is still retained.
        let (count, batch) = unsafe { (aegis_ane_feature_count(handle.as_ptr()), aegis_ane_batch_size(handle.as_ptr())) };
        let scorer = Self {
            handle,
            feature_count: count.max(0) as usize,
            batch_size: batch.max(1) as usize,
            units,
            model_path: model_path.to_path_buf(),
        };
        if scorer.feature_count != FEATURE_COUNT {
            return Err(AneError::Shape { expected: FEATURE_COUNT, got: scorer.feature_count });
        }
        Ok(scorer)
    }

    /// Default online model with `.cpuAndNeuralEngine`.
    pub fn open_default() -> Result<Self, AneError> {
        Self::open(&default_model_path(), ComputeUnits::CpuAndNeuralEngine)
    }

    /// Default batch model with `.cpuAndNeuralEngine`.
    pub fn open_batch_default() -> Result<Self, AneError> {
        Self::open(&default_batch_model_path(), ComputeUnits::CpuAndNeuralEngine)
    }

    /// Scores many operations; the bridge packs up to [`AneScorer::batch_size`] per prediction.
    pub fn score_batch(&self, vectors: &[FeatureVector]) -> Result<Vec<f32>, AneError> {
        if vectors.is_empty() {
            return Ok(Vec::new());
        }
        let mut scores = vec![0.0f32; vectors.len()];
        // SAFETY: FeatureVector is a newtype over [f32; FEATURE_COUNT], so the slice is a contiguous
        // row-major [len][FEATURE_COUNT] f32 array; `scores` has room for one value per row.
        let rc = unsafe {
            aegis_ane_score_batch(
                self.handle.as_ptr(),
                vectors.as_ptr().cast::<f32>(),
                vectors.len() as i32,
                scores.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(AneError::CoreMl(last_error()));
        }
        Ok(scores)
    }

    /// Operations per CoreML prediction (1 for the online model).
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    pub fn score(&self, features: &FeatureVector) -> Result<f32, AneError> {
        self.score_slice(features.as_slice())
    }

    /// Risk score 0..100 for a raw feature slice of length [`FEATURE_COUNT`].
    pub fn score_slice(&self, features: &[f32]) -> Result<f32, AneError> {
        if features.len() != self.feature_count {
            return Err(AneError::Shape { expected: self.feature_count, got: features.len() });
        }
        let mut score = 0.0f32;
        // SAFETY: `features` holds `feature_count` f32 values; `score` is a valid out-pointer.
        let rc = unsafe {
            aegis_ane_score_workflow(self.handle.as_ptr(), features.as_ptr(), features.len() as i32, &mut score)
        };
        if rc != 0 {
            return Err(AneError::CoreMl(last_error()));
        }
        Ok(score)
    }

    pub fn feature_count(&self) -> usize {
        self.feature_count
    }

    pub fn units(&self) -> ComputeUnits {
        self.units
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }
}

impl Drop for AneScorer {
    fn drop(&mut self) {
        // SAFETY: releases the retain taken by aegis_ane_open; the handle is not used afterwards.
        unsafe { aegis_ane_close(self.handle.as_ptr()) }
    }
}

/// Per-operation placement reported by `MLComputePlan`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlannedOp {
    pub function: String,
    pub operator: String,
    pub outputs: Vec<String>,
    /// "neural_engine", "gpu", "cpu", or "none" when CoreML does not dispatch the op (constants).
    pub preferred: String,
    pub supported: Vec<String>,
    pub estimated_cost: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComputePlanReport {
    pub compute_units: String,
    pub operations_total: usize,
    /// Operations CoreML assigns to a device (excludes constants).
    pub operations_dispatched: usize,
    pub preferred: BTreeMap<String, usize>,
    pub ops: Vec<PlannedOp>,
}

impl ComputePlanReport {
    pub fn on_neural_engine(&self) -> usize {
        self.preferred.get("neural_engine").copied().unwrap_or(0)
    }

    pub fn all_dispatched_on_neural_engine(&self) -> bool {
        self.operations_dispatched > 0 && self.on_neural_engine() == self.operations_dispatched
    }
}

/// Loads `MLComputePlan` for the compiled model and reports where each operation runs.
pub fn compute_plan(model_path: &Path, units: ComputeUnits) -> Result<ComputePlanReport, AneError> {
    if !model_path.exists() {
        return Err(AneError::ModelMissing(model_path.to_path_buf()));
    }
    let path = c_path(model_path)?;
    // SAFETY: valid C string; the returned string is owned and freed by take_string.
    let json = take_string(unsafe { aegis_ane_compute_plan_json(path.as_ptr(), units as i32) })
        .ok_or_else(|| AneError::CoreMl(last_error()))?;
    Ok(serde_json::from_str(&json)?)
}

/// `MLNeuralEngineComputeDevice.totalCoreCount`, if CoreML reports a Neural Engine.
pub fn neural_engine_cores() -> Result<Option<i64>, AneError> {
    // SAFETY: no arguments; the returned string is owned and freed by take_string.
    let json = take_string(unsafe { aegis_ane_devices_json() }).ok_or_else(|| AneError::CoreMl(last_error()))?;
    let doc: serde_json::Value = serde_json::from_str(&json)?;
    Ok(doc["devices"]
        .as_array()
        .and_then(|devices| devices.iter().find(|d| d["type"] == "neural_engine"))
        .and_then(|d| d["total_core_count"].as_i64()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn parity() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../models/parity_vectors.json")).expect("parity JSON")
    }

    #[test]
    fn matches_numpy_reference_on_parity_vectors() {
        let scorer = AneScorer::open_default().expect("model loads");
        let doc = parity();
        let tolerance = doc["tolerance_points"].as_f64().unwrap() as f32;
        let mut worst = 0.0f32;
        for (vector, reference) in doc["vectors"].as_array().unwrap().iter().zip(doc["reference_fp32"].as_array().unwrap()) {
            let v: Vec<f32> = vector.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
            let got = scorer.score_slice(&v).unwrap();
            worst = worst.max((got - reference.as_f64().unwrap() as f32).abs());
        }
        assert!(worst <= tolerance, "max |diff| {worst} exceeds {tolerance}");
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
        let expected = scorer.score(&v).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let s = Arc::clone(&scorer);
                std::thread::spawn(move || (0..200).map(|_| s.score(&v).unwrap()).collect::<Vec<_>>())
            })
            .collect();
        for h in handles {
            assert!(h.join().unwrap().iter().all(|x| *x == expected));
        }
    }

    #[test]
    fn compute_plan_lists_dispatched_operations() {
        let report = compute_plan(&default_model_path(), ComputeUnits::CpuAndNeuralEngine).unwrap();
        assert_eq!(report.compute_units, "cpuAndNeuralEngine");
        assert!(report.operations_dispatched > 0, "{report:?}");
        assert_eq!(report.preferred.values().sum::<usize>(), report.operations_dispatched);
        assert!(report.ops.iter().any(|op| op.operator.contains("linear")));
    }

    #[test]
    fn batch_model_matches_online_model() {
        let online = AneScorer::open_default().unwrap();
        let batch = AneScorer::open_batch_default().unwrap();
        assert!(batch.batch_size() > 1);
        let doc = parity();
        let vectors: Vec<FeatureVector> = doc["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let mut v = FeatureVector::default();
                for (slot, x) in v.0.iter_mut().zip(row.as_array().unwrap()) {
                    *slot = x.as_f64().unwrap() as f32;
                }
                v
            })
            .collect();
        let scores = batch.score_batch(&vectors).unwrap();
        assert_eq!(scores.len(), vectors.len());
        for (v, s) in vectors.iter().zip(&scores) {
            let single = online.score(v).unwrap();
            assert!((single - s).abs() <= 0.5, "online {single} vs batch {s}");
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
}
