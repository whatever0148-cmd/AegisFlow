// C-ABI bridge between Rust (crates/aegis-ane-scorer) and CoreML.
//
// Compiled by build.rs into a static library with `swiftc -emit-library -static`. Every exported
// symbol is `@_cdecl`; strings returned to Rust are malloc'd and must be released with
// `aegis_ane_free_string`. Errors are kept per thread and read with `aegis_ane_last_error`.
//
// Two input layouts are supported:
//   (1, C)        one operation per prediction (online scoring)
//   (1, C, 1, N)  N operations per prediction as columns (batch scoring, 1x1 convolutions)

import CoreML
import Foundation

private struct BridgeError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

private let lastErrorKey = "aegis.ane.lastError"

private func setLastError(_ message: String) {
    Thread.current.threadDictionary[lastErrorKey] = message
}

private func computeUnits(_ code: Int32) -> MLComputeUnits {
    switch code {
    case 1: return .cpuOnly
    case 2: return .cpuAndGPU
    case 3: return .cpuAndNeuralEngine
    default: return .all
    }
}

private func computeUnitsName(_ units: MLComputeUnits) -> String {
    switch units {
    case .cpuOnly: return "cpuOnly"
    case .cpuAndGPU: return "cpuAndGPU"
    case .cpuAndNeuralEngine: return "cpuAndNeuralEngine"
    case .all: return "all"
    @unknown default: return "unknown"
    }
}

private func deviceName(_ device: MLComputeDevice) -> String {
    switch device {
    case .cpu: return "cpu"
    case .gpu: return "gpu"
    case .neuralEngine: return "neural_engine"
    @unknown default: return "unknown"
    }
}

private func jsonString(_ object: Any) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    guard let text = String(data: data, encoding: .utf8) else { throw BridgeError("JSON encoding failed") }
    return text
}

/// One loaded model plus a reusable input buffer. Predictions are serialized by `lock`, so a
/// handle may be shared between threads.
private final class AegisANEModel {
    let model: MLModel
    let inputName: String
    let outputName: String
    /// Features per operation (C).
    let featureCount: Int
    /// Operations per prediction: 1 for a (1, C) input, N for a (1, C, 1, N) input.
    let batchSize: Int
    let input: MLMultiArray
    let provider: MLDictionaryFeatureProvider
    let options = MLPredictionOptions()
    let lock = NSLock()

    init(url: URL, units: MLComputeUnits) throws {
        let config = MLModelConfiguration()
        config.computeUnits = units
        model = try MLModel(contentsOf: url, configuration: config)
        let description = model.modelDescription
        guard description.inputDescriptionsByName.count == 1,
              let (name, inputDescription) = description.inputDescriptionsByName.first,
              let constraint = inputDescription.multiArrayConstraint else {
            throw BridgeError("model must have exactly one multi-array input")
        }
        guard let output = description.outputDescriptionsByName.keys.sorted().first else {
            throw BridgeError("model has no outputs")
        }
        let shape = constraint.shape.map { $0.intValue }
        if shape.count == 4 && shape[0] == 1 && shape[2] == 1 {
            featureCount = shape[1]
            batchSize = shape[3]
        } else {
            featureCount = shape.reduce(1, *)
            batchSize = 1
        }
        inputName = name
        outputName = output
        input = try MLMultiArray(shape: constraint.shape, dataType: constraint.dataType)
        provider = try MLDictionaryFeatureProvider(dictionary: [name: MLFeatureValue(multiArray: input)])
    }

    func score(_ features: UnsafePointer<Float>, count: Int) throws -> Float {
        guard count == featureCount else {
            throw BridgeError("expected \(featureCount) features, got \(count)")
        }
        var result: Float = 0
        try scoreBatch(features, operations: 1, into: &result)
        return result
    }

    /// Scores `operations` rows of `featureCount` floats, writing one score per row. Rows beyond
    /// the model's batch size are processed in further predictions.
    func scoreBatch(_ rows: UnsafePointer<Float>, operations: Int, into out: UnsafeMutablePointer<Float>) throws {
        lock.lock()
        defer { lock.unlock() }
        var done = 0
        while done < operations {
            let n = min(batchSize, operations - done)
            try fill(rows + done * featureCount, count: n)
            let result = try model.prediction(from: provider, options: options)
            guard let output = result.featureValue(for: outputName)?.multiArrayValue else {
                throw BridgeError("prediction has no multi-array output '\(outputName)'")
            }
            read(output, count: n, into: out + done)
            done += n
        }
    }

    /// Writes `count` rows into the input columns. Columns past `count` keep stale values; every
    /// column is independent (1x1 convolutions), so their outputs are simply not read.
    private func fill(_ rows: UnsafePointer<Float>, count: Int) throws {
        let strides = input.strides.map { $0.intValue }
        let featureStride = batchSize > 1 ? strides[1] : strides[strides.count - 1]
        let columnStride = batchSize > 1 ? strides[3] : 0
        let features = featureCount
        switch input.dataType {
        case .float16:
            input.withUnsafeMutableBufferPointer(ofType: Float16.self) { buffer, _ in
                for r in 0..<count {
                    for c in 0..<features { buffer[c * featureStride + r * columnStride] = Float16(rows[r * features + c]) }
                }
            }
        case .float32:
            input.withUnsafeMutableBufferPointer(ofType: Float.self) { buffer, _ in
                for r in 0..<count {
                    for c in 0..<features { buffer[c * featureStride + r * columnStride] = rows[r * features + c] }
                }
            }
        default:
            throw BridgeError("unsupported input data type \(input.dataType.rawValue)")
        }
    }

    private func read(_ output: MLMultiArray, count: Int, into out: UnsafeMutablePointer<Float>) {
        let strides = output.strides.map { $0.intValue }
        let columnStride = batchSize > 1 ? strides[strides.count - 1] : 0
        switch output.dataType {
        case .float16:
            output.withUnsafeBufferPointer(ofType: Float16.self) { b in
                for r in 0..<count { out[r] = Float(b[r * columnStride]) }
            }
        case .float32:
            output.withUnsafeBufferPointer(ofType: Float.self) { b in
                for r in 0..<count { out[r] = b[r * columnStride] }
            }
        case .double:
            output.withUnsafeBufferPointer(ofType: Double.self) { b in
                for r in 0..<count { out[r] = Float(b[r * columnStride]) }
            }
        default:
            for r in 0..<count { out[r] = output[r * columnStride].floatValue }
        }
    }
}

private final class PlanResult: @unchecked Sendable {
    var json: String?
    var error: String?
}

private func computePlanReport(url: URL, units: MLComputeUnits) throws -> String {
    let config = MLModelConfiguration()
    config.computeUnits = units
    let result = PlanResult()
    let done = DispatchSemaphore(value: 0)
    Task.detached {
        do {
            let plan = try await MLComputePlan.load(contentsOf: url, configuration: config)
            var operations: [[String: Any]] = []
            var preferred: [String: Int] = ["neural_engine": 0, "gpu": 0, "cpu": 0]
            var dispatched = 0
            func visit(_ block: MLModelStructure.Program.Block, function: String) {
                for op in block.operations {
                    let usage = plan.deviceUsage(for: op)
                    let device = usage.map { deviceName($0.preferred) } ?? "none"
                    if usage != nil {
                        dispatched += 1
                        preferred[device, default: 0] += 1
                    }
                    let cost: Any = plan.estimatedCost(of: op).map { NSNumber(value: $0.weight) } ?? NSNull()
                    operations.append([
                        "function": function,
                        "operator": op.operatorName,
                        "outputs": op.outputs.map { $0.name },
                        "preferred": device,
                        "supported": usage?.supported.map(deviceName) ?? [],
                        "estimated_cost": cost,
                    ])
                    for nested in op.blocks { visit(nested, function: function) }
                }
            }
            guard case let .program(program) = plan.modelStructure else {
                throw BridgeError("model is not an ML Program")
            }
            for name in program.functions.keys.sorted() {
                if let function = program.functions[name] { visit(function.block, function: name) }
            }
            result.json = try jsonString([
                "compute_units": computeUnitsName(units),
                "operations_total": operations.count,
                "operations_dispatched": dispatched,
                "preferred": preferred,
                "ops": operations,
            ])
        } catch {
            result.error = "\(error)"
        }
        done.signal()
    }
    done.wait()
    if let message = result.error { throw BridgeError(message) }
    guard let json = result.json else { throw BridgeError("compute plan produced no report") }
    return json
}

// MARK: - C ABI

@_cdecl("aegis_ane_open")
public func aegis_ane_open(_ path: UnsafePointer<CChar>?, _ unitsCode: Int32) -> UnsafeMutableRawPointer? {
    guard let path = path else {
        setLastError("null model path")
        return nil
    }
    do {
        let model = try AegisANEModel(url: URL(fileURLWithPath: String(cString: path)), units: computeUnits(unitsCode))
        return Unmanaged.passRetained(model).toOpaque()
    } catch {
        setLastError("open failed: \(error)")
        return nil
    }
}

@_cdecl("aegis_ane_close")
public func aegis_ane_close(_ handle: UnsafeMutableRawPointer?) {
    guard let handle = handle else { return }
    Unmanaged<AegisANEModel>.fromOpaque(handle).release()
}

@_cdecl("aegis_ane_feature_count")
public func aegis_ane_feature_count(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle = handle else { return -1 }
    return Int32(Unmanaged<AegisANEModel>.fromOpaque(handle).takeUnretainedValue().featureCount)
}

@_cdecl("aegis_ane_batch_size")
public func aegis_ane_batch_size(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle = handle else { return -1 }
    return Int32(Unmanaged<AegisANEModel>.fromOpaque(handle).takeUnretainedValue().batchSize)
}

/// Scores one operation vector. Returns 0 on success, -1 for invalid arguments, -2 when CoreML fails.
@_cdecl("aegis_ane_score_workflow")
public func aegis_ane_score_workflow(_ handle: UnsafeMutableRawPointer?, _ features: UnsafePointer<Float>?,
                                     _ count: Int32, _ outScore: UnsafeMutablePointer<Float>?) -> Int32 {
    guard let handle = handle, let features = features, let outScore = outScore, count > 0 else {
        setLastError("invalid arguments to aegis_ane_score_workflow")
        return -1
    }
    let model = Unmanaged<AegisANEModel>.fromOpaque(handle).takeUnretainedValue()
    do {
        outScore.pointee = try model.score(features, count: Int(count))
        return 0
    } catch {
        setLastError("prediction failed: \(error)")
        return -2
    }
}

/// Scores `operations` row-major vectors of `feature_count` floats into `outScores`.
@_cdecl("aegis_ane_score_batch")
public func aegis_ane_score_batch(_ handle: UnsafeMutableRawPointer?, _ rows: UnsafePointer<Float>?,
                                  _ operations: Int32, _ outScores: UnsafeMutablePointer<Float>?) -> Int32 {
    guard let handle = handle, let rows = rows, let outScores = outScores, operations > 0 else {
        setLastError("invalid arguments to aegis_ane_score_batch")
        return -1
    }
    let model = Unmanaged<AegisANEModel>.fromOpaque(handle).takeUnretainedValue()
    do {
        try model.scoreBatch(rows, operations: Int(operations), into: outScores)
        return 0
    } catch {
        setLastError("batch prediction failed: \(error)")
        return -2
    }
}

/// JSON report of MLComputePlan's preferred/supported device for every ML Program operation.
@_cdecl("aegis_ane_compute_plan_json")
public func aegis_ane_compute_plan_json(_ path: UnsafePointer<CChar>?, _ unitsCode: Int32) -> UnsafeMutablePointer<CChar>? {
    guard let path = path else {
        setLastError("null model path")
        return nil
    }
    do {
        let json = try computePlanReport(url: URL(fileURLWithPath: String(cString: path)), units: computeUnits(unitsCode))
        return strdup(json)
    } catch {
        setLastError("compute plan failed: \(error)")
        return nil
    }
}

/// JSON list of compute devices CoreML can use on this Mac (Neural Engine core count included).
@_cdecl("aegis_ane_devices_json")
public func aegis_ane_devices_json() -> UnsafeMutablePointer<CChar>? {
    var devices: [[String: Any]] = []
    for device in MLComputeDevice.allComputeDevices {
        switch device {
        case .cpu:
            devices.append(["type": "cpu"])
        case .gpu:
            devices.append(["type": "gpu"])
        case .neuralEngine(let engine):
            devices.append(["type": "neural_engine", "total_core_count": engine.totalCoreCount])
        @unknown default:
            devices.append(["type": "unknown"])
        }
    }
    do {
        return strdup(try jsonString(["devices": devices]))
    } catch {
        setLastError("devices JSON failed: \(error)")
        return nil
    }
}

@_cdecl("aegis_ane_last_error")
public func aegis_ane_last_error() -> UnsafeMutablePointer<CChar>? {
    guard let message = Thread.current.threadDictionary[lastErrorKey] as? String else { return nil }
    return strdup(message)
}

@_cdecl("aegis_ane_free_string")
public func aegis_ane_free_string(_ pointer: UnsafeMutablePointer<CChar>?) {
    free(pointer)
}
