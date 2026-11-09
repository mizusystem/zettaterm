//! UI の文言。
//!
//! - Tera Term 由来の文言（`tt` / `ssh`）は `spec/lang/*.json`（tools/extract-tt-ui が生成）をキーで引く。13 言語すべてを埋め込む
//! - アプリ独自の文言（`l` / `lf`）は日本語の文をそのままキーにして、`locale/<言語>.json` の訳を引く
//!
//! 表示言語は「その他の設定」の UI 言語（`ui_language`）、無ければ LC_ALL / LC_MESSAGES / LANG で決まる。
//! アプリ独自の文言の訳が無い言語は英語にする。

use std::collections::HashMap;
use std::sync::OnceLock;

type Sections = HashMap<String, HashMap<String, String>>;

/// 表示言語（"ja_JP" の形。spec/lang のファイル名と同じ）
pub fn lang() -> &'static str {
    static L: OnceLock<String> = OnceLock::new();
    L.get_or_init(|| {
        let chosen = crate::settings::Settings::load().other.ui_language;
        let raw = if chosen.is_empty() {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .filter_map(|k| std::env::var(k).ok())
                .find(|v| !v.is_empty())
                .unwrap_or_default()
        } else {
            chosen
        };
        let base = raw.split(['.', '@']).next().unwrap_or("");
        let code = base.split('_').next().unwrap_or("");
        let file = match (code, base) {
            ("zh", b) if b.ends_with("TW") || b.ends_with("HK") => "zh_TW",
            ("zh", _) => "zh_CN",
            ("ja", _) => "ja_JP",
            ("de", _) => "de_DE",
            ("es", _) => "es_ES",
            ("fr", _) => "fr_FR",
            ("it", _) => "it_IT",
            ("ko", _) => "ko_KR",
            ("pt", _) => "pt_BR",
            ("ru", _) => "ru_RU",
            ("ta", _) => "ta_IN",
            ("tr", _) => "tr_TR",
            _ => "en_US",
        };
        file.to_owned()
    })
}

fn spec_source(lang: &str) -> &'static str {
    match lang {
        "ja_JP" => include_str!("../../../spec/lang/ja_JP.json"),
        "de_DE" => include_str!("../../../spec/lang/de_DE.json"),
        "es_ES" => include_str!("../../../spec/lang/es_ES.json"),
        "fr_FR" => include_str!("../../../spec/lang/fr_FR.json"),
        "it_IT" => include_str!("../../../spec/lang/it_IT.json"),
        "ko_KR" => include_str!("../../../spec/lang/ko_KR.json"),
        "pt_BR" => include_str!("../../../spec/lang/pt_BR.json"),
        "ru_RU" => include_str!("../../../spec/lang/ru_RU.json"),
        "ta_IN" => include_str!("../../../spec/lang/ta_IN.json"),
        "tr_TR" => include_str!("../../../spec/lang/tr_TR.json"),
        "zh_CN" => include_str!("../../../spec/lang/zh_CN.json"),
        "zh_TW" => include_str!("../../../spec/lang/zh_TW.json"),
        _ => include_str!("../../../spec/lang/en_US.json"),
    }
}

fn sections() -> &'static Sections {
    static S: OnceLock<Sections> = OnceLock::new();
    S.get_or_init(|| {
        // 表示言語の lng。足りないキーは英語で補う
        let mut all: Sections = serde_json::from_str(spec_source("en_US")).unwrap_or_default();
        if lang() != "en_US" {
            let local: Sections = serde_json::from_str(spec_source(lang())).unwrap_or_default();
            for (section, keys) in local {
                all.entry(section).or_default().extend(keys);
            }
        }
        all
    })
}

/// アプリ独自の文言の訳（日本語 → 表示言語）。日本語のときは空
fn app_table() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| {
        if lang() == "ja_JP" {
            return HashMap::new();
        }
        // 訳を足したらここに並べる。訳の無い言語は英語
        serde_json::from_str(include_str!("../locale/en.json")).unwrap_or_default()
    })
}

/// アプリ独自の文言。日本語の文がキーで、表示言語の訳を返す（無ければ日本語のまま）。
pub fn l(ja: &str) -> String {
    app_table()
        .get(ja)
        .cloned()
        .unwrap_or_else(|| ja.to_owned())
}

/// マクロのエラー（`ファイル:行: 文`）。本家の ttpmacro は英語で出すが、日本語の表示では日本語にする。
pub fn macro_error_text(error: &str) -> String {
    if lang() == "ja_JP" {
        zetta_ttl::japanese_error(error)
    } else {
        error.to_owned()
    }
}

/// コアや OS の英語のエラーを、Tera Term の文言（表示言語のもの）か、このアプリの文言にする。
/// 元の英語の文は詳しい情報として下に添える。見分けられないものはそのまま。
/// `subject` は文に対象が無いときに入れる名前（ポートやホスト）。
pub fn error_text(message: &str, subject: Option<&str>) -> String {
    use zetta_core::error_text::{ErrorClass as C, classify};
    let info = classify(message);
    let what = info.subject.as_deref().or(subject).unwrap_or("");
    let named = |key: &str, ja: &str| tt(key, ja).replace("%s", what);
    let text = match info.class {
        C::Refused => tt("MSG_COMM_REFUSE_ERROR", "接続が拒否されました"),
        C::TimedOut => tt("MSG_COMM_TIMEOUT_ERROR", "接続がタイムアウトしました"),
        C::Unreachable => tt("MSG_COMM_REACH_ERROR", "ネットワークに到達できません"),
        C::HostNotFound => tt("MSG_INVALID_HOST_ERROR", "無効なホスト"),
        C::NotFound if what.is_empty() => tt("MSG_CANTOPEN_FILE_ERROR", "ファイルが開けません"),
        C::NotFound => named("MSG_CANTOPEN_ERROR_NOTFOUND", "%sが見つかりません。"),
        C::Denied if what.is_empty() => tt("MSG_CANTOPEN_FILE_ERROR", "ファイルが開けません"),
        C::Denied => named(
            "MSG_CANTOPEN_ERROR_DENIED",
            "%sへのアクセスが拒否されました。",
        ),
        C::ConnectionLost => l("相手が接続を切りました。"),
        C::AuthCanceled => l("認証を中止しました。"),
        C::HostKeyRejected => l("サーバーのホスト鍵を受け入れなかったため、接続を中止しました。"),
        C::HostEmpty => l("ホストを入力してください。"),
        C::SpaceParityUnsupported => {
            l("この OS では、8 ビットと space パリティの組み合わせは使えません。")
        }
        C::DsrDtrUnsupported => l("この OS では、DSR/DTR フロー制御は使えません。"),
        C::TransferRunning => l("ファイル転送の最中です。"),
        C::SpeedUnchanged => l(
            "シリアルのアダプタが速度の切り替えに応じていません。アダプタを抜いて挿し直してから、もう一度試してください。",
        ),
        C::Other => return message.to_owned(),
    };
    if text == message {
        text
    } else {
        format!("{text}\n（{message}）")
    }
}

/// 書式つき。訳の中の `{}` に `args` を順に入れる。
pub fn lf(ja: &str, args: &[&dyn std::fmt::Display]) -> String {
    let template = l(ja);
    let mut out = String::new();
    let mut rest = template.as_str();
    let mut it = args.iter();
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(a) = it.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// Tera Term の文言（"Tera Term" の節）。
pub fn tt(key: &str, fallback: &str) -> String {
    lookup("Tera Term", key, fallback)
}

/// TTSSH の文言。
pub fn ssh(key: &str, fallback: &str) -> String {
    lookup("TTSSH", key, fallback)
}

/// ほかのセクション（TTXRecurringCommand など、標準のプラグインの文言）。
pub fn section(section: &str, key: &str, fallback: &str) -> String {
    lookup(section, key, fallback)
}

fn lookup(section: &str, key: &str, fallback: &str) -> String {
    let s = sections();
    let raw = s
        .get(section)
        .and_then(|m| m.get(key))
        .or_else(|| s.get("Tera Term").and_then(|m| m.get(key)))
        .map(String::as_str)
        .unwrap_or(fallback);
    clean(raw)
}

/// Windows のアクセラレータ表記などを取り除く。
pub fn clean(s: &str) -> String {
    let mut t = s.to_owned();
    // "新しい接続(&N)...\tAlt+N" の "\t" 以降（lng では \ と t の 2 文字）
    for tab in ["\\t", "\t"] {
        if let Some(i) = t.find(tab) {
            t.truncate(i);
        }
    }
    // "(&N)" を丸ごと消す
    while let Some(i) = t.find("(&") {
        match t[i..].find(')') {
            Some(j) if j <= 4 => t.replace_range(i..i + j + 1, ""),
            _ => break,
        }
    }
    let t = t
        .replace("&&", "\u{1}")
        .replace('&', "")
        .replace('\u{1}', "&");
    let t = t.replace("\\n", "\n");
    t.strip_prefix("Tera Term: ")
        .map(str::to_owned)
        .unwrap_or(t)
}

#[cfg(test)]
mod tests {
    /// src の `l("…")` / `lf("…"` に渡した日本語の文が、すべて英語に訳してある
    #[test]
    fn every_app_string_has_an_english_translation() {
        let table: std::collections::HashMap<String, String> =
            serde_json::from_str(include_str!("../locale/en.json")).unwrap();
        let mut keys = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
            let path = entry.unwrap().path();
            // このファイル（l の定義と例文）は除く
            if path.ends_with("strings.rs") {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap();
            for marker in ["l(\"", "lf(\n        \"", "lf(\""] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(marker) {
                    // 識別子の途中（tt(… など）は除く
                    let before = rest[..i].chars().last();
                    rest = &rest[i + marker.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let end = rest.find('"').unwrap();
                    keys.insert(rest[..end].replace("\\n", "\n"));
                }
            }
        }
        assert!(keys.len() > 25, "{keys:?}");
        let missing: Vec<_> = keys.iter().filter(|k| !table.contains_key(*k)).collect();
        assert!(missing.is_empty(), "locale/en.json に訳がない: {missing:?}");
        for k in &keys {
            assert_eq!(
                k.matches("{}").count(),
                table[k].matches("{}").count(),
                "{k}"
            );
        }
    }

    #[test]
    fn format_fills_placeholders() {
        assert_eq!(super::lf("{} 件の {}", &[&3, &"x"]), "3 件の x");
    }

    #[test]
    fn clean() {
        assert_eq!(super::clean("新しい接続(&N)...\\tAlt+N"), "新しい接続...");
        assert_eq!(super::clean("T&CP/IP"), "TCP/IP");
        assert_eq!(super::clean("Tera Term: 新しい接続"), "新しい接続");
        assert_eq!(super::clean("a&&b"), "a&b");
    }
}
