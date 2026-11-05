import Foundation

/// Tera Term の UI 文言（spec/lang/*.json を build-core.sh がリソースに写したもの）。
///
/// 文言は手で書き写さず、キーで引く。`(&N)` や `\tAlt+N` などの Windows 向け表記は取り除く。
enum TT {
    private static let table: [String: String] = {
        let preferJapanese = Locale.preferredLanguages.first?.hasPrefix("ja") ?? true
        let locale = preferJapanese ? "ja_JP" : "en_US"
        guard let obj = loadJSON(locale) as? [String: [String: String]] else { return [:] }
        return obj["Tera Term"] ?? [:]
    }()

    private static let options: [String: [String]] = {
        var all: [String: [String]] = [:]
        for file in ["serial-options", "terminal-options"] {
            let lists = loadJSON(file) as? [String: [String]] ?? [:]
            all.merge(lists) { a, _ in a }
        }
        return all
    }()

    private static func loadJSON(_ name: String) -> Any? {
        guard let url = Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "spec"),
              let data = try? Data(contentsOf: url)
        else { return nil }
        return try? JSONSerialization.jsonObject(with: data)
    }

    /// キーの文言。見つからなければ `fallback`。
    static func text(_ key: String, _ fallback: String) -> String {
        clean(table[key] ?? fallback)
    }

    /// 設定の選択肢（serial_pp.cpp / term_pp.cpp の `*List[]`）。
    /// `name` は baud / data / parity / stop / flow / nl（送信改行）/ nlRcv（受信改行）。
    static func options(_ name: String) -> [String] {
        options[name] ?? []
    }

    /// Windows のアクセラレータ表記を取り除く。
    static func clean(_ s: String) -> String {
        var t = s
        // "新しい接続(&N)...\tAlt+N" の "\t" 以降（lng では \ と t の 2 文字で書かれている）
        for tab in ["\\t", "\t"] {
            if let r = t.range(of: tab) { t = String(t[..<r.lowerBound]) }
        }
        // "(&N)" を丸ごと消す
        t = t.replacingOccurrences(of: #"\(&[^)]\)"#, with: "", options: .regularExpression)
        // "T&CP/IP" の "&" を消す（"&&" は "&" として残す）
        t = t.replacingOccurrences(of: "&&", with: "\u{1}")
            .replacingOccurrences(of: "&", with: "")
            .replacingOccurrences(of: "\u{1}", with: "&")
        // "Tera Term: 新しい接続" の製品名を外す
        if t.hasPrefix("Tera Term: ") { t.removeFirst("Tera Term: ".count) }
        return t
    }
}
