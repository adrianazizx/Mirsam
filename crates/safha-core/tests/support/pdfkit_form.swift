// A form as macOS PDFKit (Preview's engine) sees it (tests/form_design.rs):
// prints each widget on the first page as "name<TAB>type<TAB>value", one per
// line. With "fill <name> <value> <out>" after the input, it first sets that
// text field's value as Preview does when a user types into it, writes the
// document to <out>, and prints the widgets of the written file.
import PDFKit

func widgets(_ document: PDFDocument) -> [PDFAnnotation] {
    guard let page = document.page(at: 0) else { return [] }
    return page.annotations.filter { $0.type == "Widget" }
}

func show(_ document: PDFDocument) {
    for widget in widgets(document) {
        let name = widget.fieldName ?? ""
        let type = widget.widgetFieldType.rawValue
        var value = widget.widgetStringValue ?? ""
        if widget.widgetFieldType == .button {
            value = widget.buttonWidgetState == .onState ? (widget.buttonWidgetStateString) : "Off"
        }
        print("\(name)\t\(type)\t\(value)")
    }
}

let args = CommandLine.arguments
let url = URL(fileURLWithPath: args[1])
guard let document = PDFDocument(url: url) else {
    FileHandle.standardError.write("cannot open \(url.path)\n".data(using: .utf8)!)
    exit(1)
}
if args.count >= 6 && args[2] == "fill" {
    let (name, value, out) = (args[3], args[4], URL(fileURLWithPath: args[5]))
    guard let field = widgets(document).first(where: { $0.fieldName == name }) else {
        FileHandle.standardError.write("no field \(name)\n".data(using: .utf8)!)
        exit(1)
    }
    field.widgetStringValue = value
    guard document.write(to: out), let written = PDFDocument(url: out) else {
        FileHandle.standardError.write("cannot write \(out.path)\n".data(using: .utf8)!)
        exit(1)
    }
    show(written)
} else {
    show(document)
}
