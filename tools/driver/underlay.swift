// A plain page under the ink layer that logs what reaches it, so a driven run can
// prove clicks, scrolls and text selection pass through an area while the draw key
// is up. The page is a text view, as a browser's text would be.
//   underlay X Y WIDTH HEIGHT [NAME]   in global top-left points; logs to stdout,
//                                      and "active NAME" each time it comes to the front
import AppKit

setvbuf(stdout, nil, _IOLBF, 0)
func log(_ text: String) { print("underlay \(text)") }

final class Page: NSTextView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) {
        log("click")
        super.mouseDown(with: event)
    }
    override func scrollWheel(with event: NSEvent) {
        log("scroll")
        super.scrollWheel(with: event)
    }
    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        let length = selectedRange().length
        if length > 0 { log("selected \(length)") }
    }
    override func insertText(_ string: Any, replacementRange: NSRange) {
        log("typed")
        super.insertText(string, replacementRange: replacementRange)
    }
}

final class Delegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    let name = CommandLine.arguments.count > 5 ? CommandLine.arguments[5] : "page"
    func applicationDidBecomeActive(_ notification: Notification) { log("active \(name)") }
    func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments.dropFirst().prefix(4).compactMap(Double.init)
        let (x, y, width, height) = (args[0], args[1], args[2], args[3])
        let primary = NSScreen.screens[0].frame.height
        window = NSWindow(
            contentRect: NSRect(x: x, y: primary - y - height, width: width, height: height),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.title = "Lens underlay \(name)"
        let page = Page(frame: NSRect(x: 0, y: 0, width: width, height: height))
        page.string = Array(repeating: "A line of plain text a person might select.", count: 60)
            .joined(separator: "\n")
        page.font = .systemFont(ofSize: 16)
        let scroll = NSScrollView(frame: page.frame)
        scroll.documentView = page
        scroll.hasVerticalScroller = true
        window.contentView = scroll
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(page)
        NSApp.activate(ignoringOtherApps: true)
        log("ready")
    }
}

let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
