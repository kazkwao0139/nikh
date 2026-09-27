// nike_ocr: PDF(스캔본) → 페이지별 OCR 텍스트 (macOS Vision, 한국어+영어, 오프라인). 사용: nike_ocr file.pdf [scale]
// 출력: 페이지마다 "\f" 로 구분한 평문. 생성 없음(문자 인식만). 파일 외 어디에도 안 보냄.
import Foundation
import PDFKit
import Vision
import AppKit

let args = CommandLine.arguments
guard args.count >= 2, let doc = PDFDocument(url: URL(fileURLWithPath: args[1])) else { FileHandle.standardError.write("usage: nike_ocr file.pdf [scale]\n".data(using: .utf8)!); exit(2) }
let scale: CGFloat = args.count >= 3 ? CGFloat(Double(args[2]) ?? 2.5) : 2.5
var out = ""
for i in 0..<doc.pageCount {
    guard let page = doc.page(at: i) else { continue }
    let bounds = page.bounds(for: .mediaBox)
    let w = Int(bounds.width * scale), h = Int(bounds.height * scale)
    guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { continue }
    ctx.setFillColor(CGColor.white); ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
    ctx.scaleBy(x: scale, y: scale); ctx.translateBy(x: -bounds.origin.x, y: -bounds.origin.y)
    page.draw(with: .mediaBox, to: ctx)
    guard let img = ctx.makeImage() else { continue }
    let req = VNRecognizeTextRequest()
    req.recognitionLevel = .accurate; req.recognitionLanguages = ["ko-KR", "en-US"]; req.usesLanguageCorrection = true
    let handler = VNImageRequestHandler(cgImage: img, options: [:])
    do { try handler.perform([req]) } catch { continue }
    let obs = (req.results ?? []).sorted { a, b in
        let ay = a.boundingBox.midY, by = b.boundingBox.midY
        if abs(ay - by) > 0.008 { return ay > by }   // 위→아래
        return a.boundingBox.minX < b.boundingBox.minX  // 좌→우
    }
    var lines: [String] = []; var lastY: CGFloat = 2; var cur: [String] = []
    for o in obs { guard let c = o.topCandidates(1).first else { continue }
        if abs(o.boundingBox.midY - lastY) > 0.008 { if !cur.isEmpty { lines.append(cur.joined(separator: " ")) }; cur = []; lastY = o.boundingBox.midY }
        cur.append(c.string) }
    if !cur.isEmpty { lines.append(cur.joined(separator: " ")) }
    out += lines.joined(separator: "\n") + "\n\u{0C}\n"
}
FileHandle.standardOutput.write(out.data(using: .utf8)!)
