// Deterministically draw NEW synthetic screen fixtures. Never capture a display.
import AppKit
import Foundation

let args = CommandLine.arguments
guard args.count == 3 else { fatalError("render-fixtures <visuals.json> <new-output-dir>") }
let data = try Data(contentsOf: URL(fileURLWithPath: args[1]))
let root = try JSONSerialization.jsonObject(with: data) as! [String: Any]
let output = URL(fileURLWithPath: args[2], isDirectory: true)
guard !FileManager.default.fileExists(atPath: output.path) else { fatalError("Output must be new") }
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
for item in root["cases"] as! [[String: Any]] {
    let id = item["id"] as! String
    let w = item["width"] as! Int, h = item["height"] as! Int
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: w * 4, bitsPerPixel: 32)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    NSColor.white.setFill()
    NSRect(x: 0, y: 0, width: w, height: h).fill()
    var top: CGFloat = 60
    for block in item["blocks"] as! [[String: Any]] {
        if let nodes = block["flow"] as? [String] {
            let nodeWidth: CGFloat = 330, nodeHeight: CGFloat = 130
            let gap = (CGFloat(w) - 120 - nodeWidth * 3) / 2
            let y = CGFloat(h) - top - nodeHeight
            let style = NSMutableParagraphStyle(); style.alignment = .center
            let attributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 27), .foregroundColor: NSColor.black, .paragraphStyle: style]
            for (index, label) in nodes.enumerated() {
                let x = 60 + CGFloat(index) * (nodeWidth + gap)
                let rect = NSRect(x: x, y: y, width: nodeWidth, height: nodeHeight)
                NSColor(calibratedWhite: 0.95, alpha: 1).setFill()
                let box = NSBezierPath(roundedRect: rect, xRadius: 14, yRadius: 14)
                box.fill(); NSColor.darkGray.setStroke(); box.lineWidth = 2; box.stroke()
                NSAttributedString(string: label, attributes: attributes).draw(in: rect.insetBy(dx: 16, dy: 28))
                if index < 2 {
                    let start = x + nodeWidth + 8, end = start + gap - 16, center = y + nodeHeight / 2
                    let arrow = NSBezierPath(); arrow.move(to: NSPoint(x: start, y: center)); arrow.line(to: NSPoint(x: end, y: center))
                    arrow.move(to: NSPoint(x: end - 15, y: center + 10)); arrow.line(to: NSPoint(x: end, y: center)); arrow.line(to: NSPoint(x: end - 15, y: center - 10)); arrow.lineWidth = 3; arrow.stroke()
                }
            }
            NSAttributedString(string: block["caption"] as! String, attributes: attributes).draw(in: NSRect(x: 60, y: y - 110, width: CGFloat(w) - 120, height: 80))
            top += nodeHeight + 170
            continue
        }
        let text = block["text"] as! String
        let size = CGFloat((block["size"] as? Int) ?? 28)
        let x = CGFloat((block["x"] as? Int) ?? 60)
        if let y = block["y"] as? Int { top = CGFloat(y) }
        let width = CGFloat((block["width"] as? Int) ?? (w - 120))
        let code = (block["code"] as? Bool) ?? false
        let font = code ? NSFont.monospacedSystemFont(ofSize: size, weight: .regular) : NSFont.systemFont(ofSize: size, weight: .regular)
        let style = NSMutableParagraphStyle()
        style.lineSpacing = size * 0.25
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor(calibratedRed: 0.09, green: 0.094, blue: 0.102, alpha: 1), .paragraphStyle: style]
        let string = NSAttributedString(string: text, attributes: attributes)
        let box = string.boundingRect(with: NSSize(width: width, height: 10000), options: [.usesLineFragmentOrigin, .usesFontLeading])
        let height = ceil(box.height) + 8
        guard x >= 0 && x + width <= CGFloat(w) && top + height <= CGFloat(h) else { fatalError("Clipped text in \(id)") }
        let rect = NSRect(x: x, y: CGFloat(h) - top - height, width: width, height: height)
        if code {
            NSColor(calibratedWhite: 0.95, alpha: 1).setFill()
            NSBezierPath(roundedRect: rect.insetBy(dx: -14, dy: -12), xRadius: 10, yRadius: 10).fill()
        }
        string.draw(with: rect, options: [.usesLineFragmentOrigin, .usesFontLeading])
        top += height + 50
    }
    NSGraphicsContext.restoreGraphicsState()
    try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(id + ".png"), options: .withoutOverwriting)
    print(id)
}
