// Test-only PDFKit observation. No PDF or system clipboard is modified.
import AppKit
import PDFKit
import CryptoKit
let args = Array(CommandLine.arguments.dropFirst())
guard let path = args.first, let bytes = try? Data(contentsOf: URL(fileURLWithPath: path)),
      let document = PDFDocument(data: bytes), document.pageCount > 0 else { exit(1) }
let margin = args.count > 1 ? Double(args[1]) ?? 44 : 44
var pages: [[String: Any]] = []
for n in 0..<document.pageCount {
    guard let page = document.page(at: n) else { exit(2) }
    let bounds = page.bounds(for: .mediaBox)
    let links: [[String: Any]] = page.annotations.compactMap { annotation in
        guard annotation.type == "Link" else { return nil }
        var item: [String: Any] = ["bounds": [annotation.bounds.minX, annotation.bounds.minY, annotation.bounds.width, annotation.bounds.height]]
        if let action = annotation.action as? PDFActionURL { item["url"] = action.url?.absoluteString ?? "" }
        if let destination = (annotation.action as? PDFActionGoTo)?.destination ?? annotation.destination, let target = destination.page {
            item["page"] = document.index(for: target)
        }
        return item
    }
    pages.append(["page": n + 1, "width": bounds.width, "height": bounds.height,
        "text": page.string ?? "", "links": links,
        "footer": page.selection(for: CGRect(x: 0, y: 0, width: bounds.width, height: margin - 6))?.string ?? ""])
}
let searches = document.findString("这是一份独立的第五组固定语料", withOptions: [])
let value: [String: Any] = ["sha256": SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined(),
    "pages": pages, "page_count": document.pageCount, "search_selections": searches.compactMap(\.string)]
let data = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
print(String(decoding: data, as: UTF8.self))
