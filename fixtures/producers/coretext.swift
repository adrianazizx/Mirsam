// Fixture producer: macOS CoreText -> Quartz PDF context.
//
// Quartz writes text differently from Chrome/Skia (glyph runs in visual order,
// its own subsetting and ToUnicode habits), which is exactly why the corpus
// needs it (docs/SPEC.md section 6.6).
//
// Usage: swift coretext.swift <content.json> <fonts.json> <fontsDir> <out.pdf>

import CoreGraphics
import CoreText
import Foundation

struct FontSpec: Decodable {
    let family: String
    let file: String
    let boldFile: String
}

struct Block: Decodable {
    let kind: String
    let dir: String?
    let font: String?
    let text: String?
    let rows: [[String]]?
    let columns: [Column]?
}

struct Column: Decodable {
    let dir: String
    let blocks: [Block]
}

struct Content: Decodable {
    let id: String
    let blocks: [Block]
}

let args = CommandLine.arguments
guard args.count == 5 else {
    FileHandle.standardError.write("usage: coretext.swift <content.json> <fonts.json> <fontsDir> <out.pdf>\n".data(using: .utf8)!)
    exit(2)
}
let content = try JSONDecoder().decode(Content.self, from: Data(contentsOf: URL(fileURLWithPath: args[1])))
let fontSpecs = try JSONDecoder().decode([String: FontSpec].self, from: Data(contentsOf: URL(fileURLWithPath: args[2])))
let fontsDir = URL(fileURLWithPath: args[3])
let outURL = URL(fileURLWithPath: args[4])

// Load fonts straight from the bundled files so the output never depends on
// (or embeds) fonts installed on this machine.
var fontCache: [String: CGFont] = [:]
func cgFont(file: String) -> CGFont {
    if let cached = fontCache[file] { return cached }
    guard let provider = CGDataProvider(url: fontsDir.appendingPathComponent(file) as CFURL),
          let font = CGFont(provider) else {
        fatalError("cannot load font \(file)")
    }
    fontCache[file] = font
    return font
}

func ctFont(key: String, size: CGFloat, bold: Bool) -> CTFont {
    guard let spec = fontSpecs[key] else { fatalError("unknown font key \(key)") }
    return CTFontCreateWithGraphicsFont(cgFont(file: bold ? spec.boldFile : spec.file), size, nil, nil)
}

let pageRect = CGRect(x: 0, y: 0, width: 595.276, height: 841.89)  // A4
let margin: CGFloat = 56
let bodySize: CGFloat = 12
let headingSize: CGFloat = 20
let gap: CGFloat = 12

var mediaBox = pageRect
guard let ctx = CGContext(outURL as CFURL, mediaBox: &mediaBox, nil) else { fatalError("cannot create PDF context") }
ctx.beginPDFPage(nil)

func attributed(_ text: String, fontKey: String, heading: Bool, rtl: Bool) -> NSAttributedString {
    var direction: CTWritingDirection = rtl ? .rightToLeft : .leftToRight
    var alignment: CTTextAlignment = .natural
    var lineSpacing: CGFloat = heading ? 4 : 5
    // CTParagraphStyleSetting stores raw pointers, so the values must stay alive
    // until CTParagraphStyleCreate has copied them.
    let style = withUnsafePointer(to: &direction) { directionPtr in
        withUnsafePointer(to: &alignment) { alignmentPtr in
            withUnsafePointer(to: &lineSpacing) { spacingPtr in
                let settings = [
                    CTParagraphStyleSetting(spec: .baseWritingDirection, valueSize: MemoryLayout<CTWritingDirection>.size, value: directionPtr),
                    CTParagraphStyleSetting(spec: .alignment, valueSize: MemoryLayout<CTTextAlignment>.size, value: alignmentPtr),
                    CTParagraphStyleSetting(spec: .lineSpacingAdjustment, valueSize: MemoryLayout<CGFloat>.size, value: spacingPtr),
                ]
                return CTParagraphStyleCreate(settings, settings.count)
            }
        }
    }
    let size = heading ? headingSize : bodySize
    let result = NSMutableAttributedString(string: text, attributes: [
        kCTFontAttributeName as NSAttributedString.Key: ctFont(key: fontKey, size: size, bold: heading),
        kCTParagraphStyleAttributeName as NSAttributedString.Key: style,
    ])
    // Latin runs inside Arabic-font text use the Latin fixture face, as a
    // browser's font fallback would.
    if fontKey != "latin" {
        let latin = ctFont(key: "latin", size: size, bold: heading)
        let ns = text as NSString
        var index = 0
        while index < ns.length {
            let ch = ns.character(at: index)
            let isLatin = (ch >= 0x41 && ch <= 0x5A) || (ch >= 0x61 && ch <= 0x7A)
            if isLatin {
                var end = index
                while end < ns.length {
                    let c = ns.character(at: end)
                    if (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A) { end += 1 } else { break }
                }
                result.addAttribute(kCTFontAttributeName as NSAttributedString.Key, value: latin, range: NSRange(location: index, length: end - index))
                index = end
            } else {
                index += 1
            }
        }
    }
    return result
}

/// Draws `text` in a box of the given width whose top edge is at `top` (PDF
/// coordinates, y up). Returns the height used.
@discardableResult
func draw(_ text: NSAttributedString, x: CGFloat, top: CGFloat, width: CGFloat, measureOnly: Bool = false) -> CGFloat {
    let setter = CTFramesetterCreateWithAttributedString(text)
    let fit = CTFramesetterSuggestFrameSizeWithConstraints(
        setter, CFRange(location: 0, length: 0), nil,
        CGSize(width: width, height: .greatestFiniteMagnitude), nil)
    let height = ceil(fit.height) + 2
    if !measureOnly {
        let path = CGPath(rect: CGRect(x: x, y: top - height, width: width, height: height), transform: nil)
        CTFrameDraw(CTFramesetterCreateFrame(setter, CFRange(location: 0, length: 0), path, nil), ctx)
    }
    return height
}

func isRTL(_ dir: String?) -> Bool { dir == "rtl" }

func hasArabic(_ text: String) -> Bool {
    text.unicodeScalars.contains { $0.value >= 0x0600 && $0.value <= 0x06FF }
}

/// Stacks text blocks downward from `top`; returns the y of the bottom edge.
func drawTextBlocks(_ blocks: [Block], x: CGFloat, top: CGFloat, width: CGFloat) -> CGFloat {
    var y = top
    for block in blocks {
        guard let text = block.text else { continue }
        let string = attributed(text, fontKey: block.font ?? "naskh", heading: block.kind == "heading", rtl: isRTL(block.dir))
        y -= draw(string, x: x, top: y, width: width) + gap
    }
    return y
}

var y = pageRect.height - margin
let contentWidth = pageRect.width - 2 * margin

for block in content.blocks {
    switch block.kind {
    case "table":
        let rows = block.rows ?? []
        let columnCount = rows.map(\.count).max() ?? 0
        guard columnCount > 0 else { continue }
        let cellWidth = contentWidth / CGFloat(columnCount)
        let padding: CGFloat = 6
        let tableRTL = isRTL(block.dir)
        for row in rows {
            let cells = row.map { cell in
                attributed(cell, fontKey: hasArabic(cell) ? (block.font ?? "naskh") : "latin", heading: false, rtl: hasArabic(cell))
            }
            let rowHeight = (cells.map { draw($0, x: 0, top: 0, width: cellWidth - 2 * padding, measureOnly: true) }.max() ?? 0) + 2 * padding
            for (index, cell) in cells.enumerated() {
                // In an RTL table the first cell sits in the rightmost column.
                let slot = tableRTL ? columnCount - 1 - index : index
                let cellX = margin + CGFloat(slot) * cellWidth
                ctx.setStrokeColor(gray: 0.6, alpha: 1)
                ctx.setLineWidth(0.5)
                ctx.stroke(CGRect(x: cellX, y: y - rowHeight, width: cellWidth, height: rowHeight))
                draw(cell, x: cellX + padding, top: y - padding, width: cellWidth - 2 * padding)
            }
            y -= rowHeight
        }
        y -= gap
    case "columns":
        let columns = block.columns ?? []
        guard !columns.isEmpty else { continue }
        let gutter: CGFloat = 24
        let columnWidth = (contentWidth - gutter * CGFloat(columns.count - 1)) / CGFloat(columns.count)
        var lowest = y
        for (index, column) in columns.enumerated() {
            let columnX = margin + CGFloat(index) * (columnWidth + gutter)
            lowest = min(lowest, drawTextBlocks(column.blocks, x: columnX, top: y, width: columnWidth))
        }
        y = lowest
    default:
        y = drawTextBlocks([block], x: margin, top: y, width: contentWidth)
    }
}

ctx.endPDFPage()
ctx.closePDF()
