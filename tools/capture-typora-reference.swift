// Opt-in calibration utility. Execute only after explicit authorization to use
// native AX/CG APIs for the Typora test window. No permission prompts are raised.
// Usage: capture-typora-reference WIDTH HEIGHT OUTPUT.png [native-blank-paragraphs.md|native-code-baselines.md|native-parity.md|native-parity-reference.md|native-nested-quotes.md|native-nested-tasks.md|native-bidi-inline.md|native-table-literals.md|native-table-struts.md|native-composed-styles.md]
import AppKit
import ApplicationServices
import CryptoKit
import ImageIO
import UniformTypeIdentifiers

struct Failure: Error { let message: String }
func require(_ value: Bool, _ message: String) throws {
    if !value { throw Failure(message: message) }
}
func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}
func setValue(_ element: AXUIElement, _ name: String, _ value: CFTypeRef) throws {
    try require(AXUIElementSetAttributeValue(element, name as CFString, value) == .success, "Cannot set \(name)")
}
let arguments = CommandLine.arguments
do {
    try require((4...5).contains(arguments.count), "Usage: capture-typora-reference WIDTH HEIGHT OUTPUT.png [native-blank-paragraphs.md|native-code-baselines.md|native-parity.md|native-parity-reference.md|native-nested-quotes.md|native-nested-tasks.md|native-bidi-inline.md|native-table-literals.md|native-table-struts.md|native-composed-styles.md]")
    guard let width = Double(arguments[1]), let height = Double(arguments[2]),
          [(900.0,620.0),(1200.0,800.0),(1600.0,1000.0)].contains(where: { $0.0 == width && $0.1 == height }) else {
        throw Failure(message: "Only the three benchmark window sizes are allowed")
    }
    let name = arguments.count == 5 ? arguments[4] : "native-parity.md"
    let fixtures = [
        "native-blank-paragraphs.md": "0ccc1297298445ad5dade414c842e70b6b5a06ee0d639d4c6f2a7afe77e90aeb",
        "native-code-baselines.md": "32b3447c753f332d3292e7479a46a03298cc8714fe8c892ccf9077f9be55319a",
        "native-parity.md": "6e65ec58e280904b5acb193a5af651eba1b462b197929daece6b727027ec6af8",
        "native-parity-reference.md": "6e65ec58e280904b5acb193a5af651eba1b462b197929daece6b727027ec6af8",
        "native-nested-quotes.md": "e2dc832997f3eab009b3b18fea83b3967d2ef66bde35d0562d9c782e53f559e5",
        "native-nested-tasks.md": "fa56d8ab537f271929ba6b1fe304d4ebb604569900db6a0580846df2ed982c6c",
        "native-bidi-inline.md": "d77f5b4c79fe2363e9b5db3ae89e3236a330b8dea59af95717bdcda355568cf4",
        "native-table-struts.md": "cc17991a103113bc8e2a3afe2509981ebf87964fe2227ed34f945b39da48cd57",
        "native-composed-styles.md": "518c6704407af5239fc1099ddb8f8f47c184031532dd6b34a0f662ccc071265f",
        "native-table-literals.md": "8e84370a35680db16139dbe5a473f5a88b8e542f322e5e99c31279fc0239c07b"
    ]
    guard let expectedHash = fixtures[name] else { throw Failure(message: "Unknown benchmark fixture") }
    let fixture = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        .appendingPathComponent("platform/macos/yu-shell-macos/Fixtures/\(name)")
    let source = try Data(contentsOf: fixture)
    let hash = SHA256.hash(data: source).map { String(format:"%02x",$0) }.joined()
    try require(hash == expectedHash, "Unexpected fixture content")
    try require(AXIsProcessTrusted(), "Accessibility unavailable; utility does not request permissions")
    try require(CGPreflightScreenCaptureAccess(), "Screen capture unavailable; utility does not request permissions")
    guard let app = NSWorkspace.shared.runningApplications.first(where: { $0.bundleIdentifier == "abnerworks.Typora" }),
          let bundleURL = app.bundleURL, let bundle = Bundle(url: bundleURL) else { throw Failure(message:"Typora must already be open") }
    try require(bundle.object(forInfoDictionaryKey:"CFBundleShortVersionString") as? String == "1.10.8", "Reference must be Typora 1.10.8")
    let ax = AXUIElementCreateApplication(app.processIdentifier)
    guard let windows = attribute(ax, kAXWindowsAttribute) as? [AXUIElement],
          let window = windows.first(where: {
              let document = attribute($0, kAXDocumentAttribute) as? String
              return (attribute($0, kAXTitleAttribute) as? String) == name
                  && document.flatMap(URL.init(string:))?.standardizedFileURL == fixture.standardizedFileURL
          }) else { throw Failure(message:"Exact benchmark test window must already be open") }
    guard let screen = NSScreen.main else { throw Failure(message:"No screen") }
    try require(screen.visibleFrame.width >= width && screen.visibleFrame.height >= height, "Benchmark size does not fit display")
    let scale = screen.backingScaleFactor
    var size = CGSize(width: width, height: height)
    var position = CGPoint(x: screen.frame.minX + 100, y: screen.frame.height - screen.visibleFrame.maxY + 40)
    guard let axSize = AXValueCreate(.cgSize, &size), let axPosition = AXValueCreate(.cgPoint, &position) else { throw Failure(message:"Cannot create geometry") }
    try setValue(window, kAXPositionAttribute, axPosition)
    try setValue(window, kAXSizeAttribute, axSize)
    try require(AXUIElementPerformAction(window,kAXRaiseAction as CFString) == .success, "Cannot raise reference window")
    app.activate(options: [])
    func currentWindow() -> [String:Any]? {
        (CGWindowListCopyWindowInfo(.optionOnScreenOnly,kCGNullWindowID) as? [[String:Any]])?.first(where: {
            ($0[kCGWindowOwnerPID as String] as? Int32) == app.processIdentifier
                && ($0[kCGWindowName as String] as? String) == name
                && ($0[kCGWindowLayer as String] as? Int) == 0
        })
    }
    let deadline = Date().addingTimeInterval(10)
    var stable = 0
    var lastBounds: [String:Double]?
    while Date() < deadline && stable < 5 {
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        let b = currentWindow()?[kCGWindowBounds as String] as? [String:Double]
        lastBounds = b
        stable = b?["Width"] == width && b?["Height"] == height ? stable + 1 : 0
    }
    guard stable == 5, let info = currentWindow(),
          let identifier = info[kCGWindowNumber as String] as? UInt32,
          let bounds = info[kCGWindowBounds as String] as? [String:Double] else {
        throw Failure(message:"Window did not settle at requested dimensions \(width)×\(height); last bounds: \(String(describing: lastBounds)); stable samples: \(stable)/5")
    }
    guard let image = CGWindowListCreateImage(.null,.optionIncludingWindow,identifier,[.boundsIgnoreFraming,.bestResolution]) else { throw Failure(message:"Capture failed") }
    try require(image.width == Int(width*scale) && image.height == Int(height*scale), "Capture is not native resolution")
    let output = URL(fileURLWithPath:arguments[3])
    try require(!FileManager.default.fileExists(atPath:output.path), "Refusing to overwrite a reference")
    try FileManager.default.createDirectory(at:output.deletingLastPathComponent(),withIntermediateDirectories:true)
    guard let destination = CGImageDestinationCreateWithURL(output as CFURL,UTType.png.identifier as CFString,1,nil) else { throw Failure(message:"Cannot create PNG") }
    CGImageDestinationAddImage(destination,image,nil)
    try require(CGImageDestinationFinalize(destination),"Cannot save PNG")
    // Native container geometry only; do not read document text or execute web code.
    func nativeChildren(_ element: AXUIElement, depth: Int = 0) -> [AXUIElement] {
        guard depth < 3 else { return [] }
        let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
        return children + children.flatMap { child in
            let role = attribute(child, kAXRoleAttribute) as? String
            return role == "AXGroup" || role == "AXScrollArea" ? nativeChildren(child, depth: depth + 1) : []
        }
    }
    let childGeometry: [[String:Any]] = nativeChildren(window).compactMap { child in
        guard let p = attribute(child, kAXPositionAttribute), CFGetTypeID(p) == AXValueGetTypeID(),
              let s = attribute(child, kAXSizeAttribute), CFGetTypeID(s) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var size = CGSize.zero
        guard AXValueGetValue(p as! AXValue, .cgPoint, &point),
              AXValueGetValue(s as! AXValue, .cgSize, &size) else { return nil }
        return ["role": attribute(child, kAXRoleAttribute) as? String ?? "unknown",
                "x": point.x - (bounds["X"] ?? 0), "y": point.y - (bounds["Y"] ?? 0),
                "width": size.width, "height": size.height]
    }
    // Version labels alone do not identify a reproducible rendering baseline.
    // Keep resource hashes, not copies of the reference application's resources.
    var resourceHashes: [String:String] = [:]
    for relative in ["TypeMark/style/base.css", "TypeMark/style/base-control.css",
                     "TypeMark/style/window.css", "TypeMark/style/themes/github.css",
                     "TypeMark/style/themes/night.css"] {
        let data = try Data(contentsOf: bundleURL.appendingPathComponent("Contents/Resources/" + relative))
        resourceHashes[relative] = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
    let webKit = Bundle(path: "/System/Library/Frameworks/WebKit.framework")
    let metadata: [String:Any] = ["app":"Typora","version":"1.10.8","source_sha256":hash,
        "app_build": bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unavailable",
        "os_version": ProcessInfo.processInfo.operatingSystemVersionString,
        "webkit_build": webKit?.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unavailable",
        "reference_resource_sha256": resourceHashes,
        "image_color_space": image.colorSpace?.name as String? ?? "unavailable",
        "window_width":width,"window_height":height,"scale":scale,"pixel_width":image.width,"pixel_height":image.height,
        "window_bounds":bounds,"native_children":childGeometry,"theme_font_zoom_scroll":"Must be verified independently through the application UI", "visual_parity_passed":false]
    try JSONSerialization.data(withJSONObject:metadata,options:[.prettyPrinted,.sortedKeys])
        .write(to:output.deletingPathExtension().appendingPathExtension("json"),options:.atomic)
    print(output.path)
} catch { fputs("\(error)\n",stderr); exit(1) }
