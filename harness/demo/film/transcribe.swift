import Foundation
import Speech
import AVFoundation

@main struct T {
  static func main() async throws {
    let url = URL(fileURLWithPath: CommandLine.arguments[1])
    let locale = Locale(identifier: "en-US")
    let tr = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange])
    if let req = try await AssetInventory.assetInstallationRequest(supporting: [tr]) {
      FileHandle.standardError.write("installing assets\n".data(using: .utf8)!)
      try await req.downloadAndInstall()
    }
    let analyzer = SpeechAnalyzer(modules: [tr])
    let file = try AVAudioFile(forReading: url)
    var out: [[String: Any]] = []
    let collect = Task {
      for try await r in tr.results {
        for run in r.text.runs {
          let s = String(r.text[run.range].characters)
          if let tr = run.audioTimeRange {
            out.append(["w": s, "s": tr.start.seconds, "e": tr.end.seconds])
          }
        }
      }
      return out
    }
    if let last = try await analyzer.analyzeSequence(from: file) {
      try await analyzer.finalizeAndFinish(through: last)
    } else { await analyzer.cancelAndFinishNow() }
    let words = try await collect.value
    let data = try JSONSerialization.data(withJSONObject: words, options: [.prettyPrinted])
    FileHandle.standardOutput.write(data)
  }
}
