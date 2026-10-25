// Placement + latency probe used by `models/build_model.py --sweep` and the final build.
//
// Compiled together with the production bridge, so latencies go through exactly the code the Rust
// crate links:
//   xcrun swiftc -O -wmo -parse-as-library crates/aegis-ane-scorer/swift/AegisANEBridge.swift \
//       models/tools/ane_probe.swift -o ane_probe
//   ane_probe MODEL.mlmodelc [...] [--rounds 40] [--per-round 60] [--warmup 300]
//
// For each model it prints one JSON line: MLComputePlan placement for .cpuAndNeuralEngine (preferred
// device per dispatched op) and per-prediction wall-clock latency of `aegis_ane_score_windows` for
// .cpuAndNeuralEngine and .cpuOnly. The two units run in alternating rounds (the unit that goes
// first alternates every round) so background load affects both alike. The input is a full window
// (a full batch of windows for batch models) of deterministic pseudo-random sparse events.

import Foundation

private struct ProbeError: Error, CustomStringConvertible {
    let description: String
}

private func lastBridgeError() -> String {
    guard let pointer = aegis_ane_last_error() else { return "unknown bridge error" }
    defer { aegis_ane_free_string(pointer) }
    return String(cString: pointer)
}

private struct Lcg {
    var state: UInt64
    mutating func next() -> Float {
        state = state &* 6364136223846793005 &+ 1442695040888963407
        return Float(state >> 40) / Float(1 << 24)
    }
}

private func percentile(_ sorted: [Double], _ q: Double) -> Double {
    sorted[Int((Double(sorted.count - 1) * q).rounded())]
}

private func summary(_ samples: [Double]) -> [String: Any] {
    let sorted = samples.sorted()
    let round6 = { (x: Double) in (x * 1e6).rounded() / 1e6 }
    return [
        "n": sorted.count,
        "p50": round6(percentile(sorted, 0.5)),
        "p95": round6(percentile(sorted, 0.95)),
        "p99": round6(percentile(sorted, 0.99)),
        "min": round6(sorted[0]),
        "mean": round6(sorted.reduce(0, +) / Double(sorted.count)),
    ]
}

private func probe(path: String, rounds: Int, perRound: Int, warmup: Int) throws -> [String: Any] {
    guard let planPointer = aegis_ane_compute_plan_json(path, 3) else { throw ProbeError(description: lastBridgeError()) }
    let planText = String(cString: planPointer)
    aegis_ane_free_string(planPointer)
    guard let plan = try JSONSerialization.jsonObject(with: Data(planText.utf8)) as? [String: Any],
          let preferred = plan["preferred"] as? [String: Int],
          let ops = plan["ops"] as? [[String: Any]] else {
        throw ProbeError(description: "unreadable compute plan JSON")
    }
    guard let ane = aegis_ane_open(path, 3) else { throw ProbeError(description: lastBridgeError()) }
    defer { aegis_ane_close(ane) }
    guard let cpu = aegis_ane_open(path, 1) else { throw ProbeError(description: lastBridgeError()) }
    defer { aegis_ane_close(cpu) }
    let features = Int(aegis_ane_feature_count(ane))
    let window = Int(aegis_ane_window_length(ane))
    let batch = Int(aegis_ane_batch_size(ane))
    var rng = Lcg(state: 0x5eed_a11c_e5)
    var events = [Float](repeating: 0, count: batch * window * features)
    for i in events.indices where rng.next() < 0.12 { events[i] = rng.next() }
    let lengths = [Int32](repeating: Int32(window), count: batch)
    var scoresAne = [Float](repeating: 0, count: batch)
    var scoresCpu = [Float](repeating: 0, count: batch)
    func run(_ handle: UnsafeMutableRawPointer, _ out: inout [Float]) throws {
        let rc = aegis_ane_score_windows(handle, events, lengths, Int32(batch), &out)
        if rc != 0 { throw ProbeError(description: lastBridgeError()) }
    }
    for _ in 0..<warmup {
        try run(ane, &scoresAne)
        try run(cpu, &scoresCpu)
    }
    var aneMs: [Double] = []
    var cpuMs: [Double] = []
    aneMs.reserveCapacity(rounds * perRound)
    cpuMs.reserveCapacity(rounds * perRound)
    for round in 0..<rounds {
        for isAne in (round % 2 == 0 ? [true, false] : [false, true]) {
            for _ in 0..<perRound {
                let start = DispatchTime.now().uptimeNanoseconds
                if isAne { try run(ane, &scoresAne) } else { try run(cpu, &scoresCpu) }
                let elapsed = Double(DispatchTime.now().uptimeNanoseconds - start) / 1e6
                if isAne { aneMs.append(elapsed) } else { cpuMs.append(elapsed) }
            }
        }
    }
    let aneSummary = summary(aneMs)
    let cpuSummary = summary(cpuMs)
    let speedup = (cpuSummary["p50"] as? Double ?? 0) / (aneSummary["p50"] as? Double ?? .nan)
    let notOnAne = ops.filter { ($0["preferred"] as? String).map { $0 != "none" && $0 != "neural_engine" } ?? false }
        .map { ["operator": $0["operator"] ?? "", "preferred": $0["preferred"] ?? ""] }
    let dispatched = plan["operations_dispatched"] as? Int ?? 0
    let onAne = preferred["neural_engine"] ?? 0
    var maxDiff: Float = 0
    for (a, c) in zip(scoresAne, scoresCpu) { maxDiff = max(maxDiff, abs(a - c)) }
    var load = [Double](repeating: 0, count: 3)
    let loadCount = getloadavg(&load, 3)
    return [
        "model": path,
        "window_len": window,
        "batch_size": batch,
        "plan": [
            "operations_total": plan["operations_total"] ?? 0,
            "operations_dispatched": dispatched,
            "neural_engine": onAne,
            "cpu": preferred["cpu"] ?? 0,
            "gpu": preferred["gpu"] ?? 0,
            "all_on_neural_engine": dispatched > 0 && onAne == dispatched,
            "not_on_neural_engine": notOnAne,
        ],
        "latency_ms": ["cpu_and_neural_engine": aneSummary, "cpu_only": cpuSummary],
        "speedup_p50_cpu_only_over_ane": (speedup * 1e4).rounded() / 1e4,
        "rounds": rounds,
        "per_round": perRound,
        "warmup": warmup,
        "max_abs_diff_between_units_points": Double(maxDiff),
        "load_average_1m_at_end": loadCount > 0 ? load[0] : -1,
    ]
}

@main
private enum AneProbe {
    static func main() {
        exit(run())
    }
}

private func run() -> Int32 {
    var models: [String] = []
    var rounds = 40, perRound = 60, warmup = 300
    var args = CommandLine.arguments.dropFirst().makeIterator()
    while let arg = args.next() {
        switch arg {
        case "--rounds", "--per-round", "--warmup":
            guard let value = args.next().flatMap({ Int($0) }), value > 0 else {
                FileHandle.standardError.write(Data("\(arg) needs a positive integer\n".utf8))
                return 2
            }
            if arg == "--rounds" { rounds = value } else if arg == "--per-round" { perRound = value } else { warmup = value }
        default:
            models.append(arg)
        }
    }
    if models.isEmpty {
        FileHandle.standardError.write(Data("usage: ane_probe MODEL.mlmodelc [...] [--rounds N] [--per-round N] [--warmup N]\n".utf8))
        return 2
    }
    var failed = false
    for path in models {
        var report: [String: Any]
        do {
            report = try probe(path: path, rounds: rounds, perRound: perRound, warmup: warmup)
        } catch {
            failed = true
            report = ["model": path, "error": "\(error)"]
        }
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]),
           let line = String(data: data, encoding: .utf8) {
            print(line)
        }
    }
    return failed ? 1 : 0
}
