// Mirsam's quick draft reader on macOS: Apple Vision (ADR 0030, ADR 0035).
//
// Built from this source on first use (`ocr::vision_helper`). Reads PNG paths from
// standard input, one per line, and answers each with one line of JSON:
//
//   {"lines":[{"text":"…","confidence":0.9,"box":[x0,y0,x1,y1],
//              "words":[{"text":"…","box":[x0,y0,x1,y1]}]}]}
//
// Boxes are in the image's pixels, origin top-left. Words are the line's text split
// at white space, each with the box Vision gives that range of the text. A page that
// cannot be read is answered {"error":"…"}.

import Foundation
import ImageIO
import Vision

func box(_ rect: CGRect, _ width: Double, _ height: Double) -> [Double] {
    // Vision's rectangles are normalized with the origin at the bottom left.
    let x0 = Double(rect.minX) * width
    let x1 = Double(rect.maxX) * width
    let y0 = (1 - Double(rect.maxY)) * height
    let y1 = (1 - Double(rect.minY)) * height
    return [x0, y0, x1, y1].map { ($0 * 10).rounded() / 10 }
}

func read(_ path: String) -> [String: Any] {
    let url = URL(fileURLWithPath: path)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else {
        return ["error": "cannot read the image"]
    }
    let (width, height) = (Double(image.width), Double(image.height))
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    request.recognitionLanguages = ["ar-SA", "en-US"]
    do {
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
    } catch {
        return ["error": "\(error)"]
    }
    var lines: [[String: Any]] = []
    for observation in request.results ?? [] {
        guard let candidate = observation.topCandidates(1).first else { continue }
        let text = candidate.string
        var words: [[String: Any]] = []
        var index = text.startIndex
        while index < text.endIndex {
            while index < text.endIndex && text[index].isWhitespace { index = text.index(after: index) }
            let start = index
            while index < text.endIndex && !text[index].isWhitespace { index = text.index(after: index) }
            if start < index {
                var word: [String: Any] = ["text": String(text[start..<index])]
                if let rect = try? candidate.boundingBox(for: start..<index)?.boundingBox {
                    word["box"] = box(rect, width, height)
                }
                words.append(word)
            }
        }
        lines.append([
            "text": text,
            "confidence": Double(candidate.confidence),
            "box": box(observation.boundingBox, width, height),
            "words": words,
        ])
    }
    return ["lines": lines]
}

setvbuf(stdout, nil, _IOLBF, 0)
while let path = readLine() {
    let answer = read(path)
    let data = (try? JSONSerialization.data(withJSONObject: answer)) ?? Data("{\"error\":\"json\"}".utf8)
    print(String(decoding: data, as: UTF8.self))
}
