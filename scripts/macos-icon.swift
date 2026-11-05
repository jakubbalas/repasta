// Draws the Repasta app icon. Usage: swift macos-icon.swift <output.png> <pixels>
import AppKit

let args = CommandLine.arguments
let size = CGFloat(Double(args[2]) ?? 1024)
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()

let inset = size * 0.1
let tile = NSRect(x: inset, y: inset, width: size - inset * 2, height: size - inset * 2)
let tilePath = NSBezierPath(roundedRect: tile, xRadius: tile.width * 0.225, yRadius: tile.width * 0.225)
NSGradient(colors: [
    NSColor(srgbRed: 0.16, green: 0.165, blue: 0.19, alpha: 1),
    NSColor(srgbRed: 0.08, green: 0.082, blue: 0.098, alpha: 1),
])!.draw(in: tilePath, angle: -90)

// Two stacked clip cards; the front one carries the accent.
let accent = NSColor(srgbRed: 0.949, green: 0.651, blue: 0.353, alpha: 1)
let unit = tile.width / 10
let back = NSRect(x: tile.minX + unit * 2.6, y: tile.minY + unit * 3.4, width: unit * 5, height: unit * 4.4)
NSColor(srgbRed: 0.3, green: 0.31, blue: 0.35, alpha: 1).setFill()
NSBezierPath(roundedRect: back, xRadius: unit * 0.7, yRadius: unit * 0.7).fill()

let front = NSRect(x: tile.minX + unit * 2.0, y: tile.minY + unit * 2.0, width: unit * 5.4, height: unit * 4.6)
accent.setFill()
NSBezierPath(roundedRect: front, xRadius: unit * 0.75, yRadius: unit * 0.75).fill()

NSColor(srgbRed: 0.08, green: 0.082, blue: 0.098, alpha: 0.85).setFill()
for (row, width) in [(0, 3.6), (1, 2.6), (2, 3.1)] {
    let line = NSRect(
        x: front.minX + unit * 0.9,
        y: front.maxY - unit * (1.3 + Double(row) * 1.05),
        width: unit * width,
        height: unit * 0.5)
    NSBezierPath(roundedRect: line, xRadius: unit * 0.25, yRadius: unit * 0.25).fill()
}

image.unlockFocus()
let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
try! bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[1]))
