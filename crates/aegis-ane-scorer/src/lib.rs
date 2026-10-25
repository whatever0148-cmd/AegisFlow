//! CoreML FP16 workflow risk scorer.
//!
//! Models are loaded through the Swift bridge in `swift/AegisANEBridge.swift` with the requested
//! `MLComputeUnits` — `.cpuAndNeuralEngine` by default. CoreML decides the final placement;
//! [`compute_plan`] reads it back from `MLComputePlan` so callers can verify where operations run.
//!
//! * `models/AegisWorkflowFP16.mlmodelc` is the online model: a temporal convolution network over
//!   a context window `(1, 64, 1, W)` — the last W events of one agent session, oldest -> newest,
//!   zero-padded on the left. It produces one risk score for the newest event in the light of the
//!   session history.
//! * `models/AegisWorkflowFP16Batch.mlmodelc` carries the same weights over `(B, 64, 1, W)` and
//!   scores B independent sessions per prediction (one per batch entry).
//!
//! Scoring semantics (identical for both models, see `models/model_card.json`):
//! * [`AneScorer::score`] / [`AneScorer::score_slice`] score one event as a session with no
//!   history (a window holding only that event).
//! * [`AneScorer::score_batch`] scores each vector the same way; `score_batch(v)[i] == score(&v[i])`
//!   up to FP16 rounding.
//! * [`AneScorer::score_window`] / [`AneScorer::score_windows`] score whole sessions: the model sees
//!   the newest [`AneScorer::window_len`] events of each [`ContextWindow`].
//!
//! The 4-D input layout is declared by the model (`aegis_input_layout` metadata written by
//! `models/build_model.py`) and reported as [`InputLayout`]; the bridge never guesses it from the
//! shape.

mod features;

pub use features::{
    commerce_features, workspace_features, CommerceContext, ExecutionContext, Feature, FeatureVector, Kind,
    FEATURE_COUNT, FEATURE_KINDS, FEATURE_NAMES,
};

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{c_char, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

extern "C" {
    fn aegis_ane_open(path: *const c_char, compute_units: i32) -> *mut c_void;
    fn aegis_ane_close(handle: *mut c_void);
    fn aegis_ane_feature_count(handle: *mut c_void) -> i32;
    fn aegis_ane_batch_size(handle: *mut c_void) -> i32;
    fn aegis_ane_window_length(handle: *mut c_void) -> i32;
    fn aegis_ane_layout(handle: *mut c_void) -> i32;
    fn aegis_ane_score_workflow(handle: *mut c_void, features: *const f32, count: i32, out_score: *mut f32) -> i32;
    fn aegis_ane_score_batch(handle: *mut c_void, rows: *const f32, operations: i32, out_scores: *mut f32) -> i32;
    fn aegis_ane_score_windows(
        handle: *mut c_void,
        events: *const f32,
        lengths: *const i32,
        sessions: i32,
        out_scores: *mut f32,
    ) -> i32;
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

/// How the model's input tensor is laid out (declared by the model, never inferred).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum InputLayout {
    /// `(1, C)`: one event per prediction.
    Vector,
    /// `(1, C, 1, N)`: N independent events as columns (phase-1 batch layout).
    Columns,
    /// `(1, C, 1, W)`: one session of the last W events, oldest -> newest, left zero-padded.
    Window,
    /// `(B, C, 1, W)`: B independent sessions along the batch axis.
    WindowBatch,
}

impl InputLayout {
    fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(InputLayout::Vector),
            1 => Some(InputLayout::Columns),
            2 => Some(InputLayout::Window),
            3 => Some(InputLayout::WindowBatch),
            _ => None,
        }
    }

    /// The metadata value (`aegis_input_layout`) that declares this layout.
    pub fn name(self) -> &'static str {
        match self {
            InputLayout::Vector => "vector",
            InputLayout::Columns => "columns",
            InputLayout::Window => "window",
            InputLayout::WindowBatch => "window_batch",
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
    #[error("context window is empty; push at least one event before scoring")]
    EmptyWindow,
    #[error("bridge returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// The most recent events of one agent session, oldest -> newest.
///
/// A ring buffer: once `capacity` events are held, [`ContextWindow::push`] evicts the oldest one.
/// Create it with `ContextWindow::new(scorer.window_len())` so it holds exactly what the model sees.
/// Shorter sessions are zero-padded on the left by the bridge (padding equals "no event").
#[derive(Debug, Clone, PartialEq)]
pub struct ContextWindow {
    events: VecDeque<FeatureVector>,
    capacity: usize,
}

impl ContextWindow {
    /// An empty window holding at most `width` events (`width` 0 is treated as 1).
    pub fn new(width: usize) -> Self {
        let capacity = width.max(1);
        Self { events: VecDeque::with_capacity(capacity), capacity }
    }

    /// Builds a window from events given oldest -> newest; only the newest `width` are kept.
    pub fn from_events<I: IntoIterator<Item = FeatureVector>>(width: usize, events: I) -> Self {
        let mut window = Self::new(width);
        for event in events {
            window.push(event);
        }
        window
    }

    /// Appends the newest event; returns the evicted oldest event when the window was full.
    pub fn push(&mut self, event: FeatureVector) -> Option<FeatureVector> {
        let evicted = if self.events.len() == self.capacity { self.events.pop_front() } else { None };
        self.events.push_back(event);
        evicted
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn newest(&self) -> Option<&FeatureVector> {
        self.events.back()
    }

    pub fn oldest(&self) -> Option<&FeatureVector> {
        self.events.front()
    }

    /// Events oldest -> newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &FeatureVector> + ExactSizeIterator {
        self.events.iter()
    }

    /// Appends the newest `limit` events (oldest -> newest) as row-major floats; returns how many.
    fn extend_newest(&self, limit: usize, out: &mut Vec<f32>) -> usize {
        let take = self.events.len().min(limit);
        for event in self.events.iter().skip(self.events.len() - take) {
            out.extend_from_slice(event.as_slice());
        }
        take
    }
}

/// Copies and frees a malloc'd string returned by the bridge.
fn take_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the bridge returns non-null, NUL-terminated strings from strdup; the pointer is read
    // here before being freed below.
    let text = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    // SAFETY: `ptr` came from strdup in the bridge and is freed exactly once, after the copy above.
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

fn ffi_len(len: usize) -> Result<i32, AneError> {
    i32::try_from(len).map_err(|_| AneError::CoreMl(format!("{len} items exceed the bridge's i32 range")))
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
    window_len: usize,
    layout: InputLayout,
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
        // SAFETY: `path` is a valid NUL-terminated C string for the duration of the call; a null
        // return is turned into an error below.
        let raw = unsafe { aegis_ane_open(path.as_ptr(), units as i32) };
        let handle = NonNull::new(raw).ok_or_else(|| AneError::CoreMl(last_error()))?;
        // SAFETY: `handle` was just returned by aegis_ane_open and stays retained until Drop; the
        // accessors only read immutable properties. Negative values (null handle) are rejected below.
        let (count, batch, window, layout) = unsafe {
            (
                aegis_ane_feature_count(handle.as_ptr()),
                aegis_ane_batch_size(handle.as_ptr()),
                aegis_ane_window_length(handle.as_ptr()),
                aegis_ane_layout(handle.as_ptr()),
            )
        };
        let layout = InputLayout::from_code(layout);
        let scorer = Self {
            handle,
            feature_count: count.max(0) as usize,
            batch_size: batch.max(1) as usize,
            window_len: window.max(1) as usize,
            layout: layout.unwrap_or(InputLayout::Vector),
            units,
            model_path: model_path.to_path_buf(),
        };
        if layout.is_none() || batch < 1 || window < 1 {
            return Err(AneError::CoreMl(format!(
                "bridge reported an invalid model description (layout {layout:?}, batch {batch}, window {window})"
            )));
        }
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

    /// Scores many events, each as a session with no history (same semantics as [`AneScorer::score`]).
    /// The bridge packs up to [`AneScorer::batch_size`] of them per CoreML prediction.
    pub fn score_batch(&self, vectors: &[FeatureVector]) -> Result<Vec<f32>, AneError> {
        if vectors.is_empty() {
            return Ok(Vec::new());
        }
        let operations = ffi_len(vectors.len())?;
        let mut scores = vec![0.0f32; vectors.len()];
        // SAFETY: FeatureVector is repr(transparent) over [f32; FEATURE_COUNT], so the slice is a
        // contiguous row-major [len][FEATURE_COUNT] f32 array, and the model's feature count was
        // checked to equal FEATURE_COUNT in `open`; `scores` has room for one value per row.
        let rc = unsafe {
            aegis_ane_score_batch(self.handle.as_ptr(), vectors.as_ptr().cast::<f32>(), operations, scores.as_mut_ptr())
        };
        if rc != 0 {
            return Err(AneError::CoreMl(last_error()));
        }
        Ok(scores)
    }

    /// Sessions per CoreML prediction (1 for the online model).
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Events of one session the model sees: W for window models, 1 for per-event models.
    pub fn window_len(&self) -> usize {
        self.window_len
    }

    pub fn layout(&self) -> InputLayout {
        self.layout
    }

    /// An empty [`ContextWindow`] sized for this model.
    pub fn new_window(&self) -> ContextWindow {
        ContextWindow::new(self.window_len)
    }

    /// Risk score 0..100 for one event as a session with no history.
    pub fn score(&self, features: &FeatureVector) -> Result<f32, AneError> {
        self.score_slice(features.as_slice())
    }

    /// Risk score 0..100 for a raw feature slice of length [`FEATURE_COUNT`], as a session with no
    /// history.
    pub fn score_slice(&self, features: &[f32]) -> Result<f32, AneError> {
        if features.len() != self.feature_count {
            return Err(AneError::Shape { expected: self.feature_count, got: features.len() });
        }
        let mut score = 0.0f32;
        // SAFETY: `features` holds exactly `feature_count` f32 values (checked above) and `score` is
        // a valid out-pointer for one f32.
        let rc = unsafe {
            aegis_ane_score_workflow(self.handle.as_ptr(), features.as_ptr(), self.feature_count as i32, &mut score)
        };
        if rc != 0 {
            return Err(AneError::CoreMl(last_error()));
        }
        Ok(score)
    }

    /// Risk score 0..100 for the newest event of `window` given the events before it. The model
    /// sees the newest [`AneScorer::window_len`] events; older ones are ignored.
    pub fn score_window(&self, window: &ContextWindow) -> Result<f32, AneError> {
        Ok(self.score_windows(std::slice::from_ref(window))?[0])
    }

    /// Scores many sessions; one score per window, packed [`AneScorer::batch_size`] per prediction.
    pub fn score_windows(&self, windows: &[ContextWindow]) -> Result<Vec<f32>, AneError> {
        if windows.is_empty() {
            return Ok(Vec::new());
        }
        if windows.iter().any(ContextWindow::is_empty) {
            return Err(AneError::EmptyWindow);
        }
        let total: usize = windows.iter().map(|w| w.len().min(self.window_len)).sum();
        let mut events = Vec::with_capacity(total * FEATURE_COUNT);
        let mut lengths = Vec::with_capacity(windows.len());
        for window in windows {
            lengths.push(ffi_len(window.extend_newest(self.window_len, &mut events))?);
        }
        let sessions = ffi_len(windows.len())?;
        let mut scores = vec![0.0f32; windows.len()];
        // SAFETY: `events` holds sum(lengths) rows of FEATURE_COUNT f32 (the model's feature count,
        // checked in `open`), every length is > 0 (empty windows rejected above), `lengths` has one
        // entry per session and `scores` has room for one f32 per session.
        let rc = unsafe {
            aegis_ane_score_windows(self.handle.as_ptr(), events.as_ptr(), lengths.as_ptr(), sessions, scores.as_mut_ptr())
        };
        if rc != 0 {
            return Err(AneError::CoreMl(last_error()));
        }
        Ok(scores)
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

    /// Dispatched operations (constants excluded), in program order.
    pub fn dispatched_ops(&self) -> impl Iterator<Item = &PlannedOp> {
        self.ops.iter().filter(|op| op.preferred != "none")
    }
}

/// Loads `MLComputePlan` for the compiled model and reports where each operation runs.
pub fn compute_plan(model_path: &Path, units: ComputeUnits) -> Result<ComputePlanReport, AneError> {
    if !model_path.exists() {
        return Err(AneError::ModelMissing(model_path.to_path_buf()));
    }
    let path = c_path(model_path)?;
    // SAFETY: `path` is a valid C string for the call; a null return is mapped to an error and a
    // non-null string is owned and freed by take_string.
    let json = take_string(unsafe { aegis_ane_compute_plan_json(path.as_ptr(), units as i32) })
        .ok_or_else(|| AneError::CoreMl(last_error()))?;
    Ok(serde_json::from_str(&json)?)
}

/// `MLNeuralEngineComputeDevice.totalCoreCount`, if CoreML reports a Neural Engine.
pub fn neural_engine_cores() -> Result<Option<i64>, AneError> {
    // SAFETY: no arguments; a null return is mapped to an error and a non-null string is owned and
    // freed by take_string.
    let json = take_string(unsafe { aegis_ane_devices_json() }).ok_or_else(|| AneError::CoreMl(last_error()))?;
    let doc: serde_json::Value = serde_json::from_str(&json)?;
    Ok(doc["devices"]
        .as_array()
        .and_then(|devices| devices.iter().find(|d| d["type"] == "neural_engine"))
        .and_then(|d| d["total_core_count"].as_i64()))
}

#[cfg(test)]
mod tests;
