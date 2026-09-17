import Foundation
import Vision
import ImageIO

let imageRoot = URL(fileURLWithPath: "/private/tmp/infinishell-gui-grok-4-bss7lqo7")
let patterns: [String: String] = [
    "provider_key": #"(?<![A-Za-z0-9_-])sk-[A-Za-z0-9_-]{16,}"#,
    "bearer_token": #"\bBearer\s+[A-Za-z0-9_.~-]{12,}"#,
    "credential_assignment": #"(?:api[-_]?key|auth[-_]?token|client[-_]?secret|password|access[-_]?token|refresh[-_]?token)[\"']?\s*[:=]\s*[\"'](?!<redacted>)[A-Za-z0-9_.~/-]{12,}"#,
    "private_api_endpoint": #"lapi[.]infinimesh[.]cloud"#,
    "private_api_file": #"/[^\s\"']*(?:api-env|api-environment)[^\s\"']*[.]json"#,
    "jwt": #"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"#,
    "private_key": #"-----BEGIN(?: [A-Z]+)? PRIVATE KEY-----"#
]
var totals = Dictionary(uniqueKeysWithValues: patterns.keys.map { ($0, 0) })
var images: [[String: Any]] = []
for file in try FileManager.default.contentsOfDirectory(atPath: imageRoot.path).filter({ $0.hasSuffix(".jpg") }).sorted() {
    let url = imageRoot.appendingPathComponent(file)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil), let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw NSError(domain: "static_image_decode", code: 1) }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = false
    request.recognitionLanguages = ["en-US", "zh-Hans"]
    try VNImageRequestHandler(cgImage: cgImage, options: [:]).perform([request])
    let segments = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    let joined = segments.joined(separator: "\n")
    let compact = joined.replacingOccurrences(of: #"\s+"#, with: "", options: .regularExpression)
    var hits: [String: Int] = [:]
    for (label, pattern) in patterns {
        let regex = try NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
        let count = [joined, compact].reduce(0) { n, text in n + regex.numberOfMatches(in: text, range: NSRange(text.startIndex..<text.endIndex, in: text)) }
        hits[label] = count
        totals[label, default: 0] += count
    }
    images.append(["file": file, "recognized_segment_count": segments.count, "credential_pattern_hits": hits])
}
guard images.count == 36 else { throw NSError(domain: "static_image_count", code: 2) }
let result: [String: Any] = ["method": "本机 Swift Vision VNRecognizeTextRequest 静态图片 OCR，accurate，关闭语言修正；逐行全文及去空白拼接文本扫描，不保存 OCR 全文", "languages": ["en-US", "zh-Hans"], "images_checked": images.count, "images": images, "credential_pattern_hits": totals, "ocr_full_text_recorded": false, "network_or_ui_operation": false]
let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys, .prettyPrinted])
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write(Data("\n".utf8))
