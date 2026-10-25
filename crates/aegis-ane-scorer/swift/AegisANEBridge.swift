// C-ABI bridge between Rust (crates/aegis-ane-scorer) and CoreML.
//
// Compiled by build.rs into a static library with `swiftc -emit-library -static`. Every exported
// symbol is `@_cdecl`; strings returned to Rust are malloc'd and must be released with
// `aegis_ane_free_string`. Errors are kept per thread and read with `aegis_ane_last_error`.
//
// Input layouts. 4-D layouts are never inferred from the shape: the model must declare them in its
// user-defined metadata under `aegis_input_layout` (models/build_model.py writes it).
//   vector        (1, C)        one event per prediction, 1 output
//   columns       (1, C, 1, N)  N independent events as columns, N outputs (phase-1 batch layout)
//   window        (1, C, 1, W)  one session: the last W events, oldest -> newest along the width,
//                               zero-padded on the left, 1 output
//   window_batch  (B, C, 1, W)  B independent sessions along the batch axis, B outputs
// A model without the key is accepted only when its input is (1, C).
//
// Everything is scored through `scoreWindows`: a list of sessions, each given as `length` events of
// C floats (oldest -> newest). Window layouts see the newest W events of each session; vector and
// columns layouts see only the newest event (their window length is 1).

import CoreML
import CoreVideo
import Foundation

private struct BridgeError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

private let lastErrorKey = "aegis.ane.lastError"
private let layoutMetadataKey = "aegis_input_layout"

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

/// Codes match `InputLayout` in src/lib.rs.
private enum InputLayout: Int32 {
    case vector = 0
    case columns = 1
    case window = 2
    case windowBatch = 3

    init?(name: String) {
        switch name {
        case "vector": self = .vector
        case "columns": self = .columns
        case "window": self = .window
        case "window_batch": self = .windowBatch
        default: return nil
        }
    }

    var name: String {
        switch self {
        case .vector: return "vector"
        case .columns: return "columns"
        case .window: return "window"
        case .windowBatch: return "window_batch"
        }
    }
}

/// Input element types the bridge writes without boxing. Float16(Float) is a single conversion
/// instruction on arm64; the generic fill functions are specialised under -wmo.
private protocol InputScalar {
    static func from(_ value: Float) -> Self
    static var zeroValue: Self { get }
}

extension Float16: InputScalar {
    @inline(__always) static func from(_ value: Float) -> Float16 { Float16(value) }
    static var zeroValue: Float16 { 0 }
}

extension Float: InputScalar {
    @inline(__always) static func from(_ value: Float) -> Float { value }
    static var zeroValue: Float { 0 }
}

/// An FP16 multi-array sharing an IOSurface-backed `kCVPixelFormatType_OneComponent16Half` pixel
/// buffer (width = last axis, height = product of the others). Returns nil for other data types or
/// when CoreVideo cannot allocate the buffer; the caller then falls back to a plain multi-array.
private func surfaceBackedArray(shape: [Int], dataType: MLMultiArrayDataType) -> MLMultiArray? {
    guard dataType == .float16, let width = shape.last, width > 0 else { return nil }
    let height = shape.dropLast().reduce(1, *)
    guard height > 0 else { return nil }
    var buffer: CVPixelBuffer?
    let attributes: [String: Any] = [kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any]()]
    let status = CVPixelBufferCreate(kCFAllocatorDefault, width, height, kCVPixelFormatType_OneComponent16Half,
                                     attributes as CFDictionary, &buffer)
    guard status == kCVReturnSuccess, let pixels = buffer else { return nil }
    return MLMultiArray(pixelBuffer: pixels, shape: shape.map { NSNumber(value: $0) })
}

/// One loaded model plus a reusable input buffer. Predictions are serialized by `lock`, so a
/// handle may be shared between threads.
private final class AegisANEModel {
    let model: MLModel
    let layout: InputLayout
    let inputName: String
    let outputName: String
    /// Features per event (C).
    let featureCount: Int
    /// Events of one session the model sees (W); 1 for vector and columns layouts.
    let windowLength: Int
    /// Sessions scored per prediction: N for columns, B for window_batch, otherwise 1.
    let unitsPerPrediction: Int
    let input: MLMultiArray
    /// Element strides of `input`: feature axis, unit axis (column or batch entry), window axis.
    let featureStride: Int
    let unitStride: Int
    let windowStride: Int
    let provider: MLDictionaryFeatureProvider
    /// Preallocated output (`MLPredictionOptions.outputBackings`), so predictions do not allocate one.
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
        guard description.outputDescriptionsByName.count == 1,
              let (output, outputDescription) = description.outputDescriptionsByName.first,
              let outputConstraint = outputDescription.multiArrayConstraint else {
            throw BridgeError("model must have exactly one multi-array output")
        }
        let shape = constraint.shape.map { $0.intValue }
        let metadata = description.metadata[MLModelMetadataKey.creatorDefinedKey] as? [String: String]
        if let declared = metadata?[layoutMetadataKey] {
            guard let parsed = InputLayout(name: declared) else {
                throw BridgeError("unknown \(layoutMetadataKey) '\(declared)' (expected vector, columns, window or window_batch)")
            }
            layout = parsed
        } else if shape.count == 2 && shape[0] == 1 {
            layout = .vector
        } else {
            throw BridgeError("input shape \(shape) needs the '\(layoutMetadataKey)' metadata key "
                + "(vector, columns, window or window_batch); rebuild with harness/build_model.sh")
        }
        switch layout {
        case .vector:
            guard shape.count == 2 && shape[0] == 1 else { throw BridgeError("vector layout needs a (1, C) input, model has \(shape)") }
            featureCount = shape[1]
            windowLength = 1
            unitsPerPrediction = 1
        case .columns:
            guard shape.count == 4 && shape[0] == 1 && shape[2] == 1 else {
                throw BridgeError("columns layout needs a (1, C, 1, N) input, model has \(shape)")
            }
            featureCount = shape[1]
            windowLength = 1
            unitsPerPrediction = shape[3]
        case .window:
            guard shape.count == 4 && shape[0] == 1 && shape[2] == 1 else {
                throw BridgeError("window layout needs a (1, C, 1, W) input, model has \(shape)")
            }
            featureCount = shape[1]
            windowLength = shape[3]
            unitsPerPrediction = 1
        case .windowBatch:
            guard shape.count == 4 && shape[2] == 1 else {
                throw BridgeError("window_batch layout needs a (B, C, 1, W) input, model has \(shape)")
            }
            featureCount = shape[1]
            unitsPerPrediction = shape[0]
            windowLength = shape[3]
        }
        guard featureCount > 0, windowLength > 0, unitsPerPrediction > 0 else {
            throw BridgeError("input shape \(shape) has an empty axis")
        }
        let outputCount = outputConstraint.shape.reduce(1) { $0 * $1.intValue }
        guard outputCount == unitsPerPrediction else {
            throw BridgeError("\(layout.name) layout scores \(unitsPerPrediction) session(s) per prediction but output "
                + "'\(output)' has shape \(outputConstraint.shape.map { $0.intValue })")
        }
        inputName = name
        outputName = output
        // FP16 inputs share an IOSurface-backed pixel buffer, which CoreML can hand to the Neural
        // Engine without an extra copy (measured ~10-15 us per prediction on M2).
        input = try surfaceBackedArray(shape: shape, dataType: constraint.dataType)
            ?? MLMultiArray(shape: constraint.shape, dataType: constraint.dataType)
        let strides = input.strides.map { $0.intValue }
        switch layout {
        case .vector:
            featureStride = strides[1]
            unitStride = 0
            windowStride = 0
        case .columns:
            featureStride = strides[1]
            unitStride = strides[3]
            windowStride = 0
        case .window:
            featureStride = strides[1]
            unitStride = 0
            windowStride = strides[3]
        case .windowBatch:
            featureStride = strides[1]
            unitStride = strides[0]
            windowStride = strides[3]
        }
        provider = try MLDictionaryFeatureProvider(dictionary: [name: MLFeatureValue(multiArray: input)])
        switch outputConstraint.dataType {
        case .float16, .float32, .double:
            options.outputBackings = [output: try MLMultiArray(shape: outputConstraint.shape, dataType: outputConstraint.dataType)]
        default:
            break
        }
    }

    /// Scores `count` sessions. Session `i` is `lengths[i]` events (all 1 when `lengths` is nil) of
    /// `featureCount` floats, oldest -> newest; sessions are stored back to back in `events`.
    func scoreWindows(_ events: UnsafePointer<Float>, lengths: UnsafePointer<Int32>?, count: Int,
                      into out: UnsafeMutablePointer<Float>) throws {
        var starts = [Int](repeating: 0, count: count)
        var sizes = [Int](repeating: 1, count: count)
        var offset = 0
        for i in 0..<count {
            let length = lengths.map { Int($0[i]) } ?? 1
            guard length > 0 else { throw BridgeError("session \(i) has no events") }
            starts[i] = offset
            sizes[i] = length
            offset += length
        }
        lock.lock()
        defer { lock.unlock() }
        var done = 0
        while done < count {
            let n = min(unitsPerPrediction, count - done)
            switch input.dataType {
            case .float16:
                input.withUnsafeMutableBufferPointer(ofType: Float16.self) { buffer, _ in
                    fill(buffer, events: events, starts: starts, sizes: sizes, first: done, count: n)
                }
            case .float32:
                input.withUnsafeMutableBufferPointer(ofType: Float.self) { buffer, _ in
                    fill(buffer, events: events, starts: starts, sizes: sizes, first: done, count: n)
                }
            default:
                throw BridgeError("unsupported input data type \(input.dataType.rawValue)")
            }
            let result = try model.prediction(from: provider, options: options)
            guard let output = result.featureValue(for: outputName)?.multiArrayValue else {
                throw BridgeError("prediction has no multi-array output '\(outputName)'")
            }
            try read(output, count: n, into: out + done)
            done += n
        }
    }

    /// Writes sessions `first ..< first + count` into units 0 ..< count of the input. Window layouts
    /// right-align the newest `windowLength` events and zero the older columns of that unit; units
    /// past `count` keep stale values, which is safe because units never mix (columns of a 1x1
    /// network, or separate batch entries) and their outputs are not read.
    private func fill<T: InputScalar>(_ buffer: UnsafeMutableBufferPointer<T>, events: UnsafePointer<Float>,
                                      starts: [Int], sizes: [Int], first: Int, count: Int) {
        guard let base = buffer.baseAddress else { return }
        let features = featureCount
        for unit in 0..<count {
            let size = sizes[first + unit]
            let used = min(size, windowLength)
            let newest = events + (starts[first + unit] + size - used) * features
            let pad = windowLength - used
            let unitBase = base + unit * unitStride
            for c in 0..<features {
                let row = unitBase + c * featureStride
                for w in 0..<pad { row[w * windowStride] = T.zeroValue }
                for i in 0..<used { row[(pad + i) * windowStride] = T.from(newest[i * features + c]) }
            }
        }
    }

    private func read(_ output: MLMultiArray, count: Int, into out: UnsafeMutablePointer<Float>) throws {
        var stride = 0
        if unitsPerPrediction > 1 {
            let shape = output.shape.map { $0.intValue }
            guard let axis = shape.firstIndex(of: unitsPerPrediction) else {
                throw BridgeError("output shape \(shape) has no axis of \(unitsPerPrediction) scores")
            }
            stride = output.strides[axis].intValue
        }
        switch output.dataType {
        case .float16:
            output.withUnsafeBufferPointer(ofType: Float16.self) { b in
                for r in 0..<count { out[r] = Float(b[r * stride]) }
            }
        case .float32:
            output.withUnsafeBufferPointer(ofType: Float.self) { b in
                for r in 0..<count { out[r] = b[r * stride] }
            }
        case .double:
            output.withUnsafeBufferPointer(ofType: Double.self) { b in
                for r in 0..<count { out[r] = Float(b[r * stride]) }
            }
        default:
            throw BridgeError("unsupported output data type \(output.dataType.rawValue)")
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

private func handleModel(_ handle: UnsafeMutableRawPointer) -> AegisANEModel {
    Unmanaged<AegisANEModel>.fromOpaque(handle).takeUnretainedValue()
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
    return Int32(handleModel(handle).featureCount)
}

/// Sessions scored per CoreML prediction (N for columns, B for window_batch, otherwise 1).
@_cdecl("aegis_ane_batch_size")
public func aegis_ane_batch_size(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle = handle else { return -1 }
    return Int32(handleModel(handle).unitsPerPrediction)
}

/// Events of one session the model sees (W for window layouts, 1 otherwise).
@_cdecl("aegis_ane_window_length")
public func aegis_ane_window_length(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle = handle else { return -1 }
    return Int32(handleModel(handle).windowLength)
}

/// 0 vector, 1 columns, 2 window, 3 window_batch; -1 for a null handle.
@_cdecl("aegis_ane_layout")
public func aegis_ane_layout(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle = handle else { return -1 }
    return handleModel(handle).layout.rawValue
}

/// Scores one event as a session with no history. Returns 0 on success, -1 for invalid arguments,
/// -2 when CoreML fails.
@_cdecl("aegis_ane_score_workflow")
public func aegis_ane_score_workflow(_ handle: UnsafeMutableRawPointer?, _ features: UnsafePointer<Float>?,
                                     _ count: Int32, _ outScore: UnsafeMutablePointer<Float>?) -> Int32 {
    guard let handle = handle, let features = features, let outScore = outScore, count > 0 else {
        setLastError("invalid arguments to aegis_ane_score_workflow")
        return -1
    }
    let model = handleModel(handle)
    guard Int(count) == model.featureCount else {
        setLastError("expected \(model.featureCount) features, got \(count)")
        return -1
    }
    do {
        try model.scoreWindows(features, lengths: nil, count: 1, into: outScore)
        return 0
    } catch {
        setLastError("prediction failed: \(error)")
        return -2
    }
}

/// Scores `operations` row-major events of `feature_count` floats, each as a session with no
/// history, into `outScores`.
@_cdecl("aegis_ane_score_batch")
public func aegis_ane_score_batch(_ handle: UnsafeMutableRawPointer?, _ rows: UnsafePointer<Float>?,
                                  _ operations: Int32, _ outScores: UnsafeMutablePointer<Float>?) -> Int32 {
    guard let handle = handle, let rows = rows, let outScores = outScores, operations > 0 else {
        setLastError("invalid arguments to aegis_ane_score_batch")
        return -1
    }
    do {
        try handleModel(handle).scoreWindows(rows, lengths: nil, count: Int(operations), into: outScores)
        return 0
    } catch {
        setLastError("batch prediction failed: \(error)")
        return -2
    }
}

/// Scores `sessions` sessions. Session i has `lengths[i]` > 0 events of `feature_count` floats,
/// oldest -> newest; sessions are concatenated in `events`. One score per session in `outScores`.
@_cdecl("aegis_ane_score_windows")
public func aegis_ane_score_windows(_ handle: UnsafeMutableRawPointer?, _ events: UnsafePointer<Float>?,
                                    _ lengths: UnsafePointer<Int32>?, _ sessions: Int32,
                                    _ outScores: UnsafeMutablePointer<Float>?) -> Int32 {
    guard let handle = handle, let events = events, let lengths = lengths, let outScores = outScores, sessions > 0 else {
        setLastError("invalid arguments to aegis_ane_score_windows")
        return -1
    }
    do {
        try handleModel(handle).scoreWindows(events, lengths: lengths, count: Int(sessions), into: outScores)
        return 0
    } catch {
        setLastError("window prediction failed: \(error)")
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
