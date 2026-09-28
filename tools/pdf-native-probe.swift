// Test-only 5B candidate probe. Public CoreText/CoreGraphics/AppKit APIs only.
// This is not a product PDF exporter or a Markdown parser.
import AppKit
import CoreText
import PDFKit
import CryptoKit

struct Input: Decodable { let title: String; let lines: [String]; let resources: [String] }
func failure(_ message: String) -> Never { fputs(message + "\n", stderr); exit(1) }
let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 2 else { failure("Usage: pdf-native-probe INPUT_JSON NEW_OUTPUT_DIRECTORY") }
let fm = FileManager.default
let inputURL = URL(fileURLWithPath: args[0])
let output = URL(fileURLWithPath: args[1], isDirectory: true)
guard !fm.fileExists(atPath: output.path) else { failure("Refusing existing output directory") }
let input = try JSONDecoder().decode(Input.self, from: Data(contentsOf: inputURL))
try fm.createDirectory(at: output, withIntermediateDirectories: false)
let bytes = NSMutableData()
var media = CGRect(x: 0, y: 0, width: 595.276, height: 841.89)
guard let consumer = CGDataConsumer(data: bytes),
      let cg = CGContext(consumer: consumer, mediaBox: &media, [kCGPDFContextTitle: input.title, kCGPDFContextCreator: "Yu 5B candidate"] as CFDictionary) else { failure("PDF context unavailable") }
var expected: [String] = []
func line(_ text: String, x: CGFloat, y: CGFloat, font: NSFont) {
    let attributed = NSAttributedString(string: text, attributes: [.font:font, .foregroundColor:NSColor.black])
    cg.textMatrix = .identity; cg.textPosition = CGPoint(x:x,y:y)
    CTLineDraw(CTLineCreateWithAttributedString(attributed), cg)
    expected.append(text)
}
cg.beginPDFPage(nil)
line(input.title, x:44,y:790,font:NSFont.boldSystemFont(ofSize:21))
for (i,text) in input.lines.enumerated() {
    line(text,x:44,y:750-CGFloat(i)*28,font:i == 1 ? NSFont.monospacedSystemFont(ofSize:13,weight:.regular) : NSFont.systemFont(ofSize:14))
}
cg.setURL(URL(string:"https://example.com/yu-pdf")! as CFURL, for: CGRect(x:44,y:655,width:300,height:22))
cg.addDestination("page-two" as CFString,at:CGPoint(x:44,y:790))
cg.endPDFPage()
var kinds: [[String:Any]]=[]
for (i,name) in input.resources.enumerated() {
    let url = inputURL.deletingLastPathComponent().appendingPathComponent(name)
    let data = try Data(contentsOf:url)
    guard let image = NSImage(data:data) else { failure("NSImage cannot read \(name)") }
    image.cacheMode = .never
    cg.beginPDFPage(nil)
    line("Resource \(i+1): \(url.lastPathComponent)",x:44,y:790,font:NSFont.systemFont(ofSize:14))
    let available=CGRect(x:44,y:80,width:507,height:660)
    let scale=min(available.width/image.size.width,available.height/image.size.height,1)
    let rect=CGRect(x:44,y:available.maxY-image.size.height*scale,width:image.size.width*scale,height:image.size.height*scale)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current=NSGraphicsContext(cgContext:cg,flipped:false)
    image.draw(in:rect,from:.zero,operation:.sourceOver,fraction:1,respectFlipped:false,hints:nil)
    NSGraphicsContext.restoreGraphicsState()
    kinds.append(["name":name,"representations":image.representations.map{String(describing:type(of:$0))},"width":rect.width,"height":rect.height])
    cg.endPDFPage()
}
cg.closePDF()
let pdf=output.appendingPathComponent("native.pdf")
try (bytes as Data).write(to:pdf,options:.withoutOverwriting)
guard let parsed=PDFDocument(url:pdf) else { failure("PDFKit could not reopen PDF") }
let text=parsed.string ?? ""
let report:[String:Any]=["pages":parsed.pageCount,"expected":expected,"extracted":text,"missing":expected.filter{!text.contains($0)},"resources":kinds,"sha256":SHA256.hash(data:bytes as Data).map{String(format:"%02x",$0)}.joined(),"evidence":"candidate-only; vector and independent viewer checks required"]
try JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]).write(to:output.appendingPathComponent("report.json"),options:.withoutOverwriting)
print(String(decoding:try JSONSerialization.data(withJSONObject:report,options:.sortedKeys),as:UTF8.self))
