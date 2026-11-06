import Foundation

/// 表示言語。OS の優先言語のうち、このアプリが対応しているもの（Info.plist の CFBundleLocalizations）。
///
/// Tera Term 由来の文言（`TT.text`）とアプリ独自の文言（`L`）の両方を、この言語にそろえる。
/// 言語を足すには `Localization/<言語>.json`（アプリ独自の文言の訳）を置き、make-app.sh の
/// CFBundleLocalizations に足す。Tera Term の文言は spec/lang の 13 言語がすでにある。
enum Lang {
    /// "ja"、"en" など
    static let code: String = {
        let supported = Bundle.main.localizations.filter { $0 != "Base" }
        if !supported.isEmpty, let first = Bundle.main.preferredLocalizations.first {
            return first
        }
        // swift run など .app でないとき
        return Locale.preferredLanguages.first.map { String($0.prefix(2)) } ?? "ja"
    }()

    /// spec/lang のファイル名（Tera Term の lng の言語）
    static var specFile: String {
        let map = [
            "ja": "ja_JP", "en": "en_US", "de": "de_DE", "es": "es_ES", "fr": "fr_FR", "it": "it_IT",
            "ko": "ko_KR", "pt": "pt_BR", "ru": "ru_RU", "ta": "ta_IN", "tr": "tr_TR",
            "zh-Hans": "zh_CN", "zh-Hant": "zh_TW",
        ]
        return map[code] ?? (code.hasPrefix("zh") ? "zh_CN" : "en_US")
    }

    /// アプリ独自の文言の訳（日本語 → この言語）。日本語のときは空
    fileprivate static let table: [String: String] = {
        guard code != "ja",
              let url = TT.resources.url(forResource: code, withExtension: "json", subdirectory: "Localization"),
              let data = try? Data(contentsOf: url),
              let dict = try? JSONSerialization.jsonObject(with: data) as? [String: String]
        else { return [:] }
        return dict
    }()
}

/// アプリ独自の文言。日本語の文をそのままキーにして、表示言語の訳を引く（無ければ日本語のまま）。
func L(_ ja: String) -> String {
    Lang.table[ja] ?? ja
}

/// 書式つき（`%@` に `args` を順に入れる）。
func L(_ ja: String, _ args: CVarArg...) -> String {
    String(format: L(ja), arguments: args)
}
