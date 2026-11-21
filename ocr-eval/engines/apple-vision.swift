// Baseline OCR engine: Apple's Vision framework (the OCR built into macOS).
//
// Not a candidate for the product's default pipeline (macOS only, closed), but a
// free, zero-install reference point: any engine we ship has to beat it.
//
// Usage: swift apple-vision.swift <manifest.json> <out-dir>
// Writes <out-dir>/<sample id>.txt for every sample in the manifest.

import Foundation
import Vision

struct Manifest: Decodable {
    struct Sample: Decodable {
        let id: String
        let image: String
    }
    let samples: [Sample]
}

let args = CommandLine.arguments
guard args.count == 3 else {
    FileHandle.standardError.write("usage: apple-vision.swift <manifest.json> <out-dir>\n".data(using: .utf8)!)
    exit(2)
}
let manifestURL = URL(fileURLWithPath: args[1])
let outDir = URL(fileURLWithPath: args[2])
let manifest = try JSONDecoder().decode(Manifest.self, from: Data(contentsOf: manifestURL))
try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

func makeRequest() -> VNRecognizeTextRequest {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    request.recognitionLanguages = ["ar-SA", "en-US"]
    return request
}

let supported = (try? makeRequest().supportedRecognitionLanguages()) ?? []
guard supported.contains("ar-SA") else {
    FileHandle.standardError.write("Vision on this macOS does not support Arabic (supported: \(supported))\n".data(using: .utf8)!)
    exit(1)
}

let started = Date()
for sample in manifest.samples {
    let imageURL = manifestURL.deletingLastPathComponent().appendingPathComponent(sample.image)
    let request = makeRequest()
    try VNImageRequestHandler(url: imageURL, options: [:]).perform([request])
    // Vision returns observations in its own reading order; keep it as-is, since
    // reading order is part of what is being measured.
    let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    try (lines.joined(separator: "\n") + "\n").write(
        to: outDir.appendingPathComponent("\(sample.id).txt"), atomically: true, encoding: .utf8)
}
let seconds = Date().timeIntervalSince(started)
print(String(format: "%d pages in %.1f s (%.1f pages/min)", manifest.samples.count, seconds, Double(manifest.samples.count) / seconds * 60))
