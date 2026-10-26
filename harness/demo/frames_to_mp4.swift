// Encodes a recorded frame sequence into a constant-frame-rate H.264 MP4 with AVAssetWriter.
//
//   swiftc -O harness/demo/frames_to_mp4.swift -o target/frames_to_mp4
//   target/frames_to_mp4 <frames-dir> <out.mp4> [fps]
//
// <frames-dir> holds frame-NNNNNN.jpg files plus frames.json: [{"file": "...", "t": seconds}, ...]
// (capture timestamps from the Chrome DevTools screencast). Each output tick shows the newest
// frame captured at or before it, so the video plays back in real time at a steady rate.
import AVFoundation
import CoreGraphics
import Foundation
import ImageIO

struct Frame: Decodable { let file: String; let t: Double }

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("frames_to_mp4: \(message)\n".utf8))
    exit(1)
}

let args = CommandLine.arguments
guard args.count >= 3 else { fail("usage: frames_to_mp4 <frames-dir> <out.mp4> [fps]") }
let dir = URL(fileURLWithPath: args[1])
let out = URL(fileURLWithPath: args[2])
let fps = args.count > 3 ? Int32(args[3]) ?? 60 : 60

guard let manifest = try? Data(contentsOf: dir.appendingPathComponent("frames.json")),
      let frames = try? JSONDecoder().decode([Frame].self, from: manifest), !frames.isEmpty
else { fail("no frames.json with frames in \(dir.path)") }

func image(_ name: String) -> CGImage {
    let url = dir.appendingPathComponent(name)
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { fail("cannot decode \(url.path)") }
    return img
}

let first = image(frames[0].file)
let (width, height) = (first.width - first.width % 2, first.height - first.height % 2)
try? FileManager.default.removeItem(at: out)
guard let writer = try? AVAssetWriter(outputURL: out, fileType: .mp4) else { fail("cannot create \(out.path)") }
let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
    AVVideoCodecKey: AVVideoCodecType.h264,
    AVVideoWidthKey: width,
    AVVideoHeightKey: height,
    AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 2_000_000, AVVideoExpectedSourceFrameRateKey: fps],
])
input.expectsMediaDataInRealTime = false
let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB,
    kCVPixelBufferWidthKey as String: width,
    kCVPixelBufferHeightKey as String: height,
])
writer.add(input)
guard writer.startWriting() else { fail("startWriting: \(writer.error.map { "\($0)" } ?? "unknown")") }
writer.startSession(atSourceTime: .zero)

func buffer(_ img: CGImage) -> CVPixelBuffer {
    var pb: CVPixelBuffer?
    guard let pool = adaptor.pixelBufferPool, CVPixelBufferPoolCreatePixelBuffer(nil, pool, &pb) == kCVReturnSuccess, let pb
    else { fail("no pixel buffer") }
    CVPixelBufferLockBaseAddress(pb, [])
    defer { CVPixelBufferUnlockBaseAddress(pb, []) }
    guard let ctx = CGContext(data: CVPixelBufferGetBaseAddress(pb), width: width, height: height, bitsPerComponent: 8,
                              bytesPerRow: CVPixelBufferGetBytesPerRow(pb), space: CGColorSpaceCreateDeviceRGB(),
                              bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue)
    else { fail("no bitmap context") }
    ctx.draw(img, in: CGRect(x: 0, y: 0, width: width, height: height))
    return pb
}

let t0 = frames[0].t
let duration = frames[frames.count - 1].t - t0
let ticks = max(1, Int((duration * Double(fps)).rounded(.up)))
var index = 0
var current = buffer(first)
for tick in 0...ticks {
    let t = t0 + Double(tick) / Double(fps)
    var advanced = false
    while index + 1 < frames.count && frames[index + 1].t <= t { index += 1; advanced = true }
    if advanced { current = buffer(image(frames[index].file)) }
    while !input.isReadyForMoreMediaData { usleep(1000) }
    if !adaptor.append(current, withPresentationTime: CMTime(value: CMTimeValue(tick), timescale: fps)) {
        fail("append failed at tick \(tick): \(writer.error.map { "\($0)" } ?? "unknown")")
    }
}
input.markAsFinished()
let done = DispatchSemaphore(value: 0)
writer.finishWriting { done.signal() }
done.wait()
guard writer.status == .completed else { fail("finishWriting: \(writer.error.map { "\($0)" } ?? "unknown")") }
print("wrote \(out.path): \(width)x\(height), \(fps) fps, \(ticks + 1) frames, \(String(format: "%.1f", duration)) s from \(frames.count) captured frames")
