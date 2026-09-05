import AppKit

let size = 1024
guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                                 bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                 colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { fatalError() }
let ctx = NSGraphicsContext(bitmapImageRep: rep)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = ctx

let bgColor = NSColor(calibratedRed: 0.06, green: 0.09, blue: 0.14, alpha: 1)
let accent = NSColor(calibratedRed: 0.31, green: 0.55, blue: 1.00, alpha: 1)

// 背景圆角矩形
bgColor.setFill()
NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: size, height: size), xRadius: 190, yRadius: 190).fill()

// 显示器外框
accent.setStroke()
let frame = NSBezierPath(roundedRect: NSRect(x: 150, y: 380, width: 724, height: 440), xRadius: 42, yRadius: 42)
frame.lineWidth = 42
frame.stroke()

// 屏幕内
NSColor(calibratedRed: 0.11, green: 0.15, blue: 0.24, alpha: 1).setFill()
NSBezierPath(roundedRect: NSRect(x: 205, y: 430, width: 614, height: 340), xRadius: 26, yRadius: 26).fill()

// 支架
accent.setStroke()
let stand = NSBezierPath()
stand.move(to: NSPoint(x: 512, y: 380))
stand.line(to: NSPoint(x: 512, y: 300))
stand.lineWidth = 34
stand.stroke()
accent.setFill()
NSBezierPath(roundedRect: NSRect(x: 400, y: 262, width: 224, height: 34), xRadius: 16, yRadius: 16).fill()

// 亮度滑块（屏幕内）
NSColor(calibratedWhite: 1.0, alpha: 0.92).setFill()
NSBezierPath(roundedRect: NSRect(x: 290, y: 540, width: 320, height: 32), xRadius: 16, yRadius: 16).fill()
NSColor(calibratedWhite: 0.2, alpha: 1).setFill()
NSBezierPath(ovalIn: NSRect(x: 570, y: 526, width: 60, height: 60)).fill()

NSGraphicsContext.restoreGraphicsState()
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: "icon.png"))
print("icon.png OK")
