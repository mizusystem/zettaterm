import Foundation
import XCTest

@testable import MultiTerm

/// アプリ独自の文言（`L("…")`）の訳し漏れを見つける。
final class LocalizationTests: XCTestCase {
    /// Sources/MultiTerm の `L("…")` に渡した日本語の文と、エラー文の対応表の日本語を集める
    private func sourceKeys() throws -> Set<String> {
        let dir = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/MultiTerm")
        var keys = Set<String>()
        // L( の直後の文字列リテラル（改行をはさんでもよい）
        let call = try NSRegularExpression(pattern: #"\bL\(\s*"((?:[^"\\]|\\.)*)""#)
        // TerminalController.localized の ("english", "日本語") の対
        let pair = try NSRegularExpression(pattern: #"\("[^"]+",\s*"((?:[^"\\]|\\.)*[ぁ-んァ-ヶ一-龠](?:[^"\\]|\\.)*)"\)"#)
        for file in try FileManager.default.contentsOfDirectory(atPath: dir.path) where file.hasSuffix(".swift") {
            let text = try String(contentsOf: dir.appendingPathComponent(file), encoding: .utf8)
            let range = NSRange(text.startIndex..., in: text)
            for re in [call] + (file == "TerminalController.swift" ? [pair] : []) {
                for m in re.matches(in: text, range: range) {
                    guard let r = Range(m.range(at: 1), in: text) else { continue }
                    keys.insert(String(text[r]).replacingOccurrences(of: "\\n", with: "\n"))
                }
            }
        }
        return keys
    }

    func testEveryAppStringHasAnEnglishTranslation() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/MultiTerm/Localization/en.json")
        let table = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: String] ?? [:]
        let keys = try sourceKeys()
        XCTAssertGreaterThan(keys.count, 40)
        let missing = keys.filter { table[$0] == nil }.sorted()
        XCTAssertEqual(missing, [], "en.json に訳がない")
        // 書式の %@ の数が同じ
        for key in keys {
            guard let en = table[key] else { continue }
            XCTAssertEqual(key.components(separatedBy: "%@").count, en.components(separatedBy: "%@").count, key)
        }
    }

    func testJapaneseIsTheSourceText() {
        // 訳が無い文はそのまま返る
        XCTAssertEqual(L("存在しない文"), "存在しない文")
        XCTAssertEqual(L("%@ 件", "3"), "3 件")
    }
}
