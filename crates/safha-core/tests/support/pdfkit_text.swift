// Prints a PDF's first page text as macOS PDFKit (Preview's engine) reads it:
// the "another viewer" of spec section 7's acceptance tests (tests/m3_exit.rs).
import PDFKit

let url = URL(fileURLWithPath: CommandLine.arguments[1])
guard let document = PDFDocument(url: url), let page = document.page(at: 0) else {
    FileHandle.standardError.write("cannot open \(url.path)\n".data(using: .utf8)!)
    exit(1)
}
print(page.string ?? "")
