import AppKit
import CoreGraphics

let srcPath = "/Users/nanyu/Library/Application Support/Hermes/composer-images/composer_2026-09-04_08-44-31-739_6eae33.png"
guard let img = NSImage(contentsOfFile: srcPath),
      let tiff = img.tiffRepresentation,
      let rep = NSBitmapImageRep(data: tiff) else { fatalError("载入失败") }

let w = rep.pixelsWide, h = rep.pixelsHigh
guard let data = rep.bitmapData else { fatalError("无像素") }
let bpr = rep.bytesPerRow, bpp = rep.bitsPerPixel / 8

// 1) 找蓝色图标 bbox
var minX = w, maxX = -1, minY = h, maxY = -1
for y in 0..<h {
    for x in 0..<w {
        let off = y * bpr + x * bpp
        let r = Int(data[off]), g = Int(data[off+1]), b = Int(data[off+2])
        if b > 150 && b > r + 40 && b > g + 25 {
            if x < minX { minX = x }; if x > maxX { maxX = x }
            if y < minY { minY = y }; if y > maxY { maxY = y }
        }
    }
}
guard maxX > minX && maxY > minY else { fatalError("未检测到蓝色图标") }
let cw = maxX - minX + 1, ch = maxY - minY + 1
print("bbox \(cw)x\(ch)")

// 2) 取裁剪 CGImage（CGImage cropping 用左上原点，与 bitmap 扫描一致）
let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil)!
guard let cropped = cg.cropping(to: CGRect(x: minX, y: minY, width: cw, height: ch)) else { fatalError("裁剪失败") }

// 3) 读出裁剪像素，把非蓝色/非深色的米黄背景变透明
var px = [UInt8](repeating: 0, count: cw * ch * 4)
let cs = CGColorSpaceCreateDeviceRGB()
let rc = CGContext(data: &px, width: cw, height: ch, bitsPerComponent: 8, bytesPerRow: cw*4, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
rc.draw(cropped, in: CGRect(x: 0, y: 0, width: cw, height: ch))
for i in stride(from: 0, to: px.count, by: 4) {
    let r = Int(px[i]), g = Int(px[i+1]), b = Int(px[i+2])
    let blue = b > r + 20
    let dark = max(r, g, b) < 95
    px[i+3] = (blue || dark) ? 255 : 0
}
let rc2 = CGContext(data: &px, width: cw, height: ch, bitsPerComponent: 8, bytesPerRow: cw*4, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
guard let trans = rc2.makeImage() else { fatalError("透明图失败") }

// 4) 缩放到 1024 并铺到透明画布
let out = 1024
var oc = [UInt8](repeating: 0, count: out * out * 4)
let octx = CGContext(data: &oc, width: out, height: out, bitsPerComponent: 8, bytesPerRow: out*4, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
octx.interpolationQuality = .high
octx.draw(trans, in: CGRect(x: 0, y: 0, width: out, height: out))
guard let outImg = octx.makeImage() else { fatalError("输出失败") }
let outRep = NSBitmapImageRep(cgImage: outImg)
guard let png = outRep.representation(using: .png, properties: [:]) else { fatalError("写png") }
try! png.write(to: URL(fileURLWithPath: "/Users/nanyu/screenguard-tauri/icon.png"))
print("icon.png transparent 1024 OK")
