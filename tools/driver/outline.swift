// Proves the area's outline and corner handles stay out of real stills: counts the
// pixels near the outline's blue (#0a84ff) in a band along each edge of each PNG.
//   outline FRAME.png ...      prints one line per file; exits 1 if any has such pixels
import AppKit

// The outline is 2 pt and a handle's dot sits within 24 pt of a corner: the band
// covers both at a 2× still.
let band = 48
var found = false
for path in CommandLine.arguments.dropFirst() {
    guard let image = NSImage(contentsOfFile: path),
        let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil)
    else {
        print("outline: cannot read \(path)")
        exit(2)
    }
    let (width, height) = (cg.width, cg.height)
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    let context = CGContext(
        data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
    var blue = 0
    for y in 0..<height {
        for x in 0..<width where x < band || y < band || x >= width - band || y >= height - band {
            let i = (y * width + x) * 4
            let (r, g, b) = (Int(pixels[i]), Int(pixels[i + 1]), Int(pixels[i + 2]))
            if abs(r - 10) < 40 && abs(g - 132) < 40 && b > 215 { blue += 1 }
        }
    }
    print("outline: \(path) \(width)×\(height), \(blue) outline-blue pixels along the edges")
    if blue > 0 { found = true }
}
exit(found ? 1 : 0)
