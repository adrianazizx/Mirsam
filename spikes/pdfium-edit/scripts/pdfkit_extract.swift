// Second, independent text extractor: macOS PDFKit (what Preview and Safari use).
//   swift pdfkit_extract.swift a.pdf "b.pdf|password" ...
// Prints { "<file stem>": ["page 0 text", ...] } as JSON, first four pages only.
import Foundation
import PDFKit

var result: [String: [String]] = [:]
for argument in CommandLine.arguments.dropFirst() {
    let parts = argument.split(separator: "|", maxSplits: 1).map(String.init)
    let url = URL(fileURLWithPath: parts[0])
    guard let document = PDFDocument(url: url) else {
        FileHandle.standardError.write("PDFKit could not open \(parts[0])\n".data(using: .utf8)!)
        continue
    }
    if document.isLocked, parts.count > 1 {
        _ = document.unlock(withPassword: parts[1])
    }
    if document.isLocked {
        FileHandle.standardError.write("PDFKit could not unlock \(parts[0])\n".data(using: .utf8)!)
        continue
    }
    var pages: [String] = []
    for index in 0..<min(document.pageCount, 4) {
        pages.append(document.page(at: index)?.string ?? "")
    }
    result[url.deletingPathExtension().lastPathComponent] = pages
}
let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
FileHandle.standardOutput.write(data)
