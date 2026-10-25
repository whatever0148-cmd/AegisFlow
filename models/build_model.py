#!/usr/bin/env python3
"""Train, export and compile the AegisWorkflowFP16 CoreML risk classifier.

Deterministic pipeline (seed in TRAIN_CONFIG):
  1. sample synthetic operations (workspace edits, build failures, orders, uniform noise) over the
     64 features declared in feature_schema.json
  2. label them with the heuristic teacher declared in the same file (linear weights + pairwise
     interactions -> sigmoid -> 0..100)
  3. train a 64-32-16-1 MLP (ReLU, sigmoid x 100) with NumPy + Adam
  4. build an FP16 ML Program with coremltools' MIL builder (fp16 input/output, macOS 14 opset)
  5. save AegisWorkflowFP16.mlpackage and compile it to AegisWorkflowFP16.mlmodelc
  6. compare CoreML FP16 predictions with the NumPy FP32 forward pass and write parity_vectors.json
     and model_card.json

The teacher is a hand-written heuristic: the model reproduces it and demonstrates the on-device
pipeline; it has not been validated on real incident data.

Usage:  .venv/bin/python models/build_model.py [--verify-only]
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import shutil
import sys
import time
from pathlib import Path
from typing import Dict, List, Tuple

import numpy as np

HERE = Path(__file__).resolve().parent
SCHEMA_PATH = HERE / "feature_schema.json"
MLPACKAGE = HERE / "AegisWorkflowFP16.mlpackage"
MLMODELC = HERE / "AegisWorkflowFP16.mlmodelc"
BATCH_MLPACKAGE = HERE / "AegisWorkflowFP16Batch.mlpackage"
BATCH_MLMODELC = HERE / "AegisWorkflowFP16Batch.mlmodelc"
PARITY = HERE / "parity_vectors.json"
CARD = HERE / "model_card.json"

# The batch model applies the same weights as 1x1 convolutions over (1, 64, 1, BATCH) so many
# operations are scored in one prediction. CoreML's MLComputePlan keeps small workloads on the CPU
# even with .cpuAndNeuralEngine; probes on Apple M2 / macOS 27.0.1 placed this network on the
# Neural Engine at 16384 columns (4096 stayed on the CPU). Re-check placement on other hardware
# with the compute-plan report in artifacts/aegis_phase1_telemetry.json.
BATCH = 16384

TRAIN_CONFIG = {
    "seed": 20261025,
    "train_samples": 80000,
    "val_samples": 20000,
    "hidden": [32, 16],
    "epochs": 45,
    "batch_size": 256,
    "lr": 3e-3,
    "lr_late": 8e-4,
    "lr_switch_epoch": 32,
    "scenario_mix": {"workspace_edit": 0.40, "build_failure": 0.20, "order": 0.30, "uniform": 0.10},
}
PARITY_TOLERANCE = 1.0  # score points (0..100 scale)

# Base probability that a feature of a group is active in each scenario. Each sample also draws an
# intensity u ~ Beta(1.3, 3.5); the effective probability is base * (0.25 + 1.5 * u), so most
# operations touch few risk signals and a minority touch many.
P_ACTIVE = {
    "workspace_edit": {"change_footprint": 0.35, "critical_paths": 0.06, "content_signals": 0.06,
                       "execution_context": 0.25, "commerce": 0.0},
    "build_failure": {"change_footprint": 0.30, "critical_paths": 0.08, "content_signals": 0.06,
                      "execution_context": 0.40, "commerce": 0.0},
    "order": {"change_footprint": 0.0, "critical_paths": 0.0, "content_signals": 0.05,
              "execution_context": 0.15, "commerce": 0.30},
}
# Features present in every operation of a scenario (an edit always touches a file, an order always
# has an amount).
ALWAYS = {
    "workspace_edit": ["files_touched", "bytes_changed", "lines_added", "agent_uncertainty"],
    "build_failure": ["files_touched", "bytes_changed", "lines_added", "agent_uncertainty"],
    "order": ["order_amount_cents", "line_items", "quantity_total", "budget_utilization"],
}
UNIFORM_DENSITY = 0.12
BINARY_RATE = 0.25  # binary flags (symlink change, manual override, ...) are rarer than graded measures


def load_schema() -> Dict:
    schema = json.loads(SCHEMA_PATH.read_text())
    feats = schema["features"]
    assert len(feats) == schema["feature_count"] == 64, "schema must declare 64 features"
    assert [f["index"] for f in feats] == list(range(64)), "feature indices must be 0..63 in order"
    return schema


def sample_values(rng: np.random.Generator, feat: Dict, n: int) -> np.ndarray:
    kind = feat["kind"]
    if kind == "binary":
        return np.ones(n, dtype=np.float32)
    if kind == "ratio":
        return rng.beta(1.5, 3.0, size=n).astype(np.float32)
    if kind == "count":
        cap = float(feat["cap"])
        k = rng.poisson(max(cap * 0.15, 1.0), size=n) + 1
        return (np.minimum(k, cap) / cap).astype(np.float32)
    if kind == "log":
        return rng.beta(2.0, 2.5, size=n).astype(np.float32)
    raise ValueError(f"unknown feature kind {kind}")


def sample_dataset(rng: np.random.Generator, n: int, schema: Dict) -> np.ndarray:
    feats = schema["features"]
    idx = {f["name"]: f["index"] for f in feats}
    mix = TRAIN_CONFIG["scenario_mix"]
    names = list(mix.keys())
    scenario = rng.choice(len(names), size=n, p=[mix[k] for k in names])
    X = np.zeros((n, 64), dtype=np.float32)
    for s_i, s_name in enumerate(names):
        rows = np.where(scenario == s_i)[0]
        if rows.size == 0:
            continue
        if s_name == "uniform":
            block = rng.random((rows.size, 64)).astype(np.float32)
            for f in feats:
                if f["kind"] == "binary":
                    block[:, f["index"]] = (block[:, f["index"]] > 0.5).astype(np.float32)
            block *= (rng.random((rows.size, 64)) < UNIFORM_DENSITY)
            X[rows] = block
            continue
        intensity = 0.25 + 1.5 * rng.beta(1.3, 3.5, size=rows.size)
        for f in feats:
            base = P_ACTIVE[s_name][f["group"]] * (BINARY_RATE if f["kind"] == "binary" else 1.0)
            if f["name"] in ("test_files_ratio", "docs_files_ratio") and s_name != "order":
                base = 0.4
            if f["name"] in ALWAYS[s_name]:
                active = rows
            else:
                active = rows[rng.random(rows.size) < np.minimum(base * intensity, 1.0)]
            if active.size:
                X[active, f["index"]] = sample_values(rng, f, active.size)
        if s_name == "build_failure":
            X[rows, idx["build_failed"]] = (rng.random(rows.size) < 0.9).astype(np.float32)
    return X


def teacher(X: np.ndarray, schema: Dict) -> np.ndarray:
    """Heuristic label in (0, 1); risk score = 100 * label."""
    w = np.array([f["w"] for f in schema["features"]], dtype=np.float64)
    z = X.astype(np.float64) @ w
    idx = {f["name"]: f["index"] for f in schema["features"]}
    for it in schema["interactions"]:
        z += it["w"] * X[:, idx[it["a"]]] * X[:, idx[it["b"]]]
    t = schema["teacher"]
    return 1.0 / (1.0 + np.exp(-t["slope"] * (z - t["bias"])))


# ---------------------------------------------------------------------------------------------
# NumPy MLP
# ---------------------------------------------------------------------------------------------
def init_params(rng: np.random.Generator, sizes: List[int]) -> List[np.ndarray]:
    params: List[np.ndarray] = []
    for fan_in, fan_out in zip(sizes[:-1], sizes[1:]):
        params.append(rng.normal(0.0, np.sqrt(2.0 / fan_in), size=(fan_out, fan_in)).astype(np.float32))
        params.append(np.zeros(fan_out, dtype=np.float32))
    return params


def forward(params: List[np.ndarray], X: np.ndarray):
    W1, b1, W2, b2, W3, b3 = params
    a1 = X @ W1.T + b1
    h1 = np.maximum(a1, 0.0)
    a2 = h1 @ W2.T + b2
    h2 = np.maximum(a2, 0.0)
    z = h2 @ W3.T + b3
    p = 1.0 / (1.0 + np.exp(-z))
    return p[:, 0], (X, a1, h1, a2, h2, p)


def backward(params: List[np.ndarray], cache, y: np.ndarray) -> List[np.ndarray]:
    _, _, W2, _, W3, _ = params
    X, a1, h1, a2, h2, p = cache
    dz = (p[:, 0] - y)[:, None] / X.shape[0]  # d(BCE)/d(logit)
    dW3, db3 = dz.T @ h2, dz.sum(0)
    da2 = (dz @ W3) * (a2 > 0)
    dW2, db2 = da2.T @ h1, da2.sum(0)
    da1 = (da2 @ W2) * (a1 > 0)
    dW1, db1 = da1.T @ X, da1.sum(0)
    return [dW1, db1, dW2, db2, dW3, db3]


def train(X: np.ndarray, y: np.ndarray, Xv: np.ndarray, yv: np.ndarray, rng: np.random.Generator):
    cfg = TRAIN_CONFIG
    params = init_params(rng, [64] + cfg["hidden"] + [1])
    m = [np.zeros_like(p) for p in params]
    v = [np.zeros_like(p) for p in params]
    step = 0
    history = []
    for epoch in range(cfg["epochs"]):
        lr = cfg["lr"] if epoch < cfg["lr_switch_epoch"] else cfg["lr_late"]
        order = rng.permutation(X.shape[0])
        for start in range(0, X.shape[0], cfg["batch_size"]):
            batch = order[start:start + cfg["batch_size"]]
            _, cache = forward(params, X[batch])
            grads = backward(params, cache, y[batch].astype(np.float32))
            step += 1
            for i, g in enumerate(grads):
                m[i] = 0.9 * m[i] + 0.1 * g
                v[i] = 0.999 * v[i] + 0.001 * (g * g)
                m_hat = m[i] / (1 - 0.9 ** step)
                v_hat = v[i] / (1 - 0.999 ** step)
                params[i] -= (lr * m_hat / (np.sqrt(v_hat) + 1e-8)).astype(np.float32)
        if epoch % 5 == 4 or epoch == cfg["epochs"] - 1:
            pv, _ = forward(params, Xv)
            mae = float(np.mean(np.abs(pv - yv)) * 100.0)
            history.append({"epoch": epoch + 1, "val_mae_points": round(mae, 4)})
            print(f"  epoch {epoch + 1:3d}  val MAE {mae:.3f} score points", flush=True)
    return params, history


def classification(pred: np.ndarray, target: np.ndarray, threshold: float) -> Dict[str, float]:
    p, t = pred >= threshold, target >= threshold
    tp, fp, fn = int(np.sum(p & t)), int(np.sum(p & ~t)), int(np.sum(~p & t))
    return {
        "accuracy": round(float(np.mean(p == t)), 5),
        "precision": round(tp / (tp + fp), 5) if tp + fp else None,
        "recall": round(tp / (tp + fn), 5) if tp + fn else None,
        "positive_rate_teacher": round(float(np.mean(t)), 5),
    }


# ---------------------------------------------------------------------------------------------
# CoreML export
# ---------------------------------------------------------------------------------------------
def export_coreml(params: List[np.ndarray], schema: Dict, metrics: Dict, batch: int = 1):
    """batch == 1: (1, 64) input with linear layers. batch > 1: (1, 64, 1, batch) with 1x1 convs."""
    import coremltools as ct
    from coremltools.converters.mil import Builder as mb
    from coremltools.converters.mil.mil import types

    W1, b1, W2, b2, W3, b3 = [p.astype(np.float16) for p in params]
    in_name, out_name = schema["input_name"], schema["output_name"]

    if batch == 1:
        @mb.program(input_specs=[mb.TensorSpec(shape=(1, 64), dtype=types.fp16)], opset_version=ct.target.macOS14)
        def prog(features):
            h1 = mb.relu(x=mb.linear(x=features, weight=W1, bias=b1, name="dense_1"), name="relu_1")
            h2 = mb.relu(x=mb.linear(x=h1, weight=W2, bias=b2, name="dense_2"), name="relu_2")
            prob = mb.sigmoid(x=mb.linear(x=h2, weight=W3, bias=b3, name="logit"), name="probability")
            return mb.mul(x=prob, y=np.float16(100.0), name=out_name)
    else:
        k1, k2, k3 = (w.reshape(w.shape[0], w.shape[1], 1, 1) for w in (W1, W2, W3))

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, 64, 1, batch), dtype=types.fp16)],
                    opset_version=ct.target.macOS14)
        def prog(features):
            h1 = mb.relu(x=mb.conv(x=features, weight=k1, bias=b1, name="dense_1"), name="relu_1")
            h2 = mb.relu(x=mb.conv(x=h1, weight=k2, bias=b2, name="dense_2"), name="relu_2")
            prob = mb.sigmoid(x=mb.conv(x=h2, weight=k3, bias=b3, name="logit"), name="probability")
            return mb.mul(x=prob, y=np.float16(100.0), name=out_name)

    assert in_name == "features", "MIL program argument name defines the CoreML input name"
    model = ct.convert(prog, convert_to="mlprogram", minimum_deployment_target=ct.target.macOS14,
                       compute_precision=ct.precision.FLOAT16, compute_units=ct.ComputeUnit.CPU_AND_NE)
    model.author = "AegisFlow harness (models/build_model.py)"
    model.short_description = ("AegisFlow workflow complexity/risk scorer: 64 normalized operation features -> "
                               "risk_score 0..100. FP16 ML Program. Trained on synthetic data labelled by a "
                               "documented heuristic teacher.")
    model.version = f"{schema['version']}.0"
    if batch == 1:
        model.input_description[in_name] = "64 normalized features, order defined by models/feature_schema.json"
        model.output_description[out_name] = "risk score 0..100 (>= 35 is high impact)"
    else:
        model.input_description[in_name] = (f"{batch} operations as columns: [0, feature, 0, operation], "
                                            "feature order defined by models/feature_schema.json")
        model.output_description[out_name] = "risk score 0..100 per operation column"
    model.user_defined_metadata.update({
        "feature_schema_sha256": hashlib.sha256(SCHEMA_PATH.read_bytes()).hexdigest(),
        "val_mae_points": str(metrics["val_mae_points"]),
        "seed": str(TRAIN_CONFIG["seed"]),
    })
    return model


def weights_digest(params: List[np.ndarray]) -> str:
    h = hashlib.sha256()
    for p in params:
        h.update(p.astype(np.float16).tobytes())
    return h.hexdigest()


def coreml_predict(model_path: Path, vectors: np.ndarray, compute_units) -> np.ndarray:
    import coremltools as ct
    model = ct.models.MLModel(str(model_path), compute_units=compute_units)
    out = []
    for vec in vectors:
        res = model.predict({"features": vec.reshape(1, 64).astype(np.float16)})
        out.append(float(np.asarray(res["risk_score"]).reshape(-1)[0]))
    return np.array(out, dtype=np.float64)


def coreml_predict_batch(model_path: Path, vectors: np.ndarray, compute_units) -> np.ndarray:
    import coremltools as ct
    model = ct.models.MLModel(str(model_path), compute_units=compute_units)
    columns = np.zeros((1, 64, 1, BATCH), dtype=np.float16)
    columns[0, :, 0, :len(vectors)] = vectors.T.astype(np.float16)
    res = model.predict({"features": columns})
    return np.asarray(res["risk_score"], dtype=np.float64).reshape(-1)[:len(vectors)]


def parity_set(rng: np.random.Generator, Xv: np.ndarray) -> np.ndarray:
    picks = Xv[rng.choice(Xv.shape[0], size=224, replace=False)]
    extremes = [np.zeros(64, dtype=np.float32), np.ones(64, dtype=np.float32)]
    for start in range(0, 64, 2):  # pairs of adjacent features fully on
        e = np.zeros(64, dtype=np.float32)
        e[start:start + 2] = 1.0
        extremes.append(e)
    return np.vstack([picks, np.array(extremes[:32])]).astype(np.float32)


def verify_only() -> int:
    import coremltools as ct
    problems = []
    for path in (MLPACKAGE, MLMODELC, BATCH_MLPACKAGE, BATCH_MLMODELC, PARITY, CARD):
        if not path.exists():
            problems.append(f"missing {path.name}")
    if problems:
        print("verify: " + "; ".join(problems) + " (run harness/build_model.sh)")
        return 1
    parity = json.loads(PARITY.read_text())
    vectors = np.array(parity["vectors"], dtype=np.float32)
    ref = np.array(parity["reference_fp32"], dtype=np.float64)
    got = coreml_predict(MLPACKAGE, vectors, ct.ComputeUnit.CPU_AND_NE)
    diff = np.abs(got - ref)
    batch_diff = np.abs(coreml_predict_batch(BATCH_MLPACKAGE, vectors, ct.ComputeUnit.CPU_AND_NE) - ref)
    print(f"verify: {len(vectors)} vectors, CoreML FP16 vs NumPy FP32 max |diff| {diff.max():.4f}, "
          f"mean {diff.mean():.4f}; batch model max |diff| {batch_diff.max():.4f} (tolerance {PARITY_TOLERANCE})")
    diff = np.maximum(diff, batch_diff)
    spec = ct.models.MLModel(str(MLPACKAGE), skip_model_load=True).get_spec()
    in_type = spec.description.input[0].type.multiArrayType
    ok_shape = list(in_type.shape) == [1, 64]
    print(f"verify: input {spec.description.input[0].name} shape {list(in_type.shape)} dataType {in_type.dataType}; "
          f"output {spec.description.output[0].name}; mlmodelc present: {MLMODELC.exists()}")
    return 0 if (diff.max() <= PARITY_TOLERANCE and ok_shape) else 1


def main() -> int:
    ap = argparse.ArgumentParser(description="build the AegisWorkflowFP16 CoreML model")
    ap.add_argument("--verify-only", action="store_true", help="check existing artifacts, do not retrain")
    args = ap.parse_args()
    if args.verify_only:
        return verify_only()

    import coremltools as ct
    from coremltools.models.utils import compile_model

    t_start = time.time()
    schema = load_schema()
    rng = np.random.default_rng(TRAIN_CONFIG["seed"])
    X = sample_dataset(rng, TRAIN_CONFIG["train_samples"], schema)
    Xv = sample_dataset(rng, TRAIN_CONFIG["val_samples"], schema)
    y, yv = teacher(X, schema), teacher(Xv, schema)
    thr = schema["high_impact_threshold"]
    print(f"dataset: {X.shape[0]} train / {Xv.shape[0]} val, teacher high-impact rate "
          f"{np.mean(yv * 100 >= thr):.3f}")

    params, history = train(X, y, Xv, yv, rng)
    pv, _ = forward(params, Xv)
    val_pred, val_true = pv * 100.0, yv * 100.0
    metrics = {
        "val_mae_points": round(float(np.mean(np.abs(val_pred - val_true))), 4),
        "val_rmse_points": round(float(np.sqrt(np.mean((val_pred - val_true) ** 2))), 4),
        "high_impact_vs_teacher": classification(val_pred, val_true, thr),
        "history": history,
    }
    print(f"trained: val MAE {metrics['val_mae_points']} pts, RMSE {metrics['val_rmse_points']} pts, "
          f"high-impact agreement {metrics['high_impact_vs_teacher']}")

    for batch, package, compiled_path in ((1, MLPACKAGE, MLMODELC), (BATCH, BATCH_MLPACKAGE, BATCH_MLMODELC)):
        model = export_coreml(params, schema, metrics, batch=batch)
        if package.exists():
            shutil.rmtree(package)
        model.save(str(package))
        if compiled_path.exists():
            shutil.rmtree(compiled_path)
        compiled = Path(compile_model(str(package), destination_path=str(compiled_path)))
        print(f"saved {package.name}; compiled -> {compiled.name}")

    vectors = parity_set(rng, Xv)
    ref, _ = forward(params, vectors)
    ref = ref.astype(np.float64) * 100.0
    coreml_ne = coreml_predict(MLPACKAGE, vectors, ct.ComputeUnit.CPU_AND_NE)
    coreml_cpu = coreml_predict(MLPACKAGE, vectors, ct.ComputeUnit.CPU_ONLY)
    coreml_batch = coreml_predict_batch(BATCH_MLPACKAGE, vectors, ct.ComputeUnit.CPU_AND_NE)
    diff = np.abs(coreml_ne - ref)
    batch_diff = np.abs(coreml_batch - ref)
    parity = {
        "description": "NumPy FP32 forward pass of the exported weights vs CoreML FP16 predictions (coremltools).",
        "feature_schema_sha256": hashlib.sha256(SCHEMA_PATH.read_bytes()).hexdigest(),
        "tolerance_points": PARITY_TOLERANCE,
        "vectors": [[round(float(v), 6) for v in row] for row in vectors],
        "reference_fp32": [round(float(v), 5) for v in ref],
        "coreml_fp16_cpu_and_ne": [round(float(v), 5) for v in coreml_ne],
        "coreml_fp16_cpu_only": [round(float(v), 5) for v in coreml_cpu],
        "coreml_fp16_batch_cpu_and_ne": [round(float(v), 5) for v in coreml_batch],
        "teacher": [round(float(v) * 100.0, 5) for v in teacher(vectors, schema)],
    }
    PARITY.write_text(json.dumps(parity) + "\n")
    print(f"parity: max |CoreML FP16 - NumPy FP32| = {diff.max():.4f} pts (mean {diff.mean():.4f}); "
          f"batch model {batch_diff.max():.4f} pts, over {len(vectors)} vectors")

    card = {
        "model": MLPACKAGE.name,
        "compiled": MLMODELC.name,
        "batch_model": {"package": BATCH_MLPACKAGE.name, "compiled": BATCH_MLMODELC.name,
                        "input_shape": [1, 64, 1, BATCH], "max_abs_diff_points": round(float(batch_diff.max()), 5)},
        "built_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "architecture": "linear(64->32) relu linear(32->16) relu linear(16->1) sigmoid x100",
        "precision": "float16 weights, activations, input and output (ML Program, macOS 14 opset)",
        "parameters": int(sum(p.size for p in params)),
        "weights_fp16_sha256": weights_digest(params),
        "training": TRAIN_CONFIG,
        "metrics": metrics,
        "parity": {"vectors": len(vectors), "max_abs_diff_points": round(float(diff.max()), 5),
                   "mean_abs_diff_points": round(float(diff.mean()), 5)},
        "toolchain": {"python": sys.version.split()[0], "numpy": np.__version__, "coremltools": ct.__version__},
        "limitations": [
            "Labels come from the heuristic teacher in feature_schema.json, not from real incidents.",
            "Scores are only as meaningful as the feature extraction feeding them.",
        ],
        "build_seconds": round(time.time() - t_start, 2),
    }
    CARD.write_text(json.dumps(card, indent=2) + "\n")
    print(f"model card -> {CARD.name}")
    return 0 if max(diff.max(), batch_diff.max()) <= PARITY_TOLERANCE else 1


if __name__ == "__main__":
    sys.exit(main())
