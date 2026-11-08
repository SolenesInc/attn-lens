// Writes the editor check's synthetic image and puts it on the clipboard: one line of
// black text on white above a solid block of one colour, 800x400 pixels.
//   editor-fixture OUT.png
import AppKit

let text = "Attn Lens reads this line 42"
let block: (CGFloat, CGFloat, CGFloat) = (0x1F, 0x7A, 0x4D)  // #1F7A4D
let (width, height) = (800, 400)

guard CommandLine.arguments.count == 2 else {
    print("usage: editor-fixture OUT.png")
    exit(2)
}
let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0)!.retagging(with: .sRGB)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSColor.white.setFill()
NSRect(x: 0, y: 0, width: width, height: height).fill()
NSColor(srgbRed: block.0 / 255, green: block.1 / 255, blue: block.2 / 255, alpha: 1).setFill()
NSRect(x: 0, y: 0, width: width, height: 250).fill()
(text as NSString).draw(
    at: NSPoint(x: 40, y: 300),
    withAttributes: [.font: NSFont.systemFont(ofSize: 44), .foregroundColor: NSColor.black])
NSGraphicsContext.restoreGraphicsState()

let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
NSPasteboard.general.clearContents()
NSPasteboard.general.setData(png, forType: .png)
print("fixture \(width)x\(height) text=\"\(text)\" block=#1F7A4D")
