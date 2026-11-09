//! マクロのエラーの日本語訳。
//!
//! インタプリタのエラーは英語の文（`no label 'foo'` など）で、UI には `ファイル:行: 文` の形で届く。
//! 本家の ttpmacro は英語で出すが、日本語の表示では読めるようにする（英語の表示は今のまま）。
//! 文の型（`{0}` が差し込み）と訳を並べ、上から順に当てる。具体的なものを先に置く。

use std::sync::OnceLock;

use regex::Regex;

/// (英語の型, 日本語)。`{0}` `{1}` は差し込み。
const TABLE: &[(&str, &str)] = &[
    // コマンドごとのもの（下の汎用の型より先に）
    ("logrotate: count expected", "logrotate: 世代数が必要です"),
    (
        "logrotate: count must be 1 or more",
        "logrotate: 世代数は 1 以上にしてください",
    ),
    ("logrotate: size expected", "logrotate: サイズが必要です"),
    (
        "logrotate: size must be larger than 128",
        "logrotate: サイズは 128 より大きくしてください",
    ),
    ("logrotate: unknown '{0}'", "logrotate: '{0}' は使えません"),
    (
        "strreplace: index is inside a character",
        "strreplace: 位置が文字の途中です",
    ),
    ("waitregex: no pattern", "waitregex: パターンがありません"),
    ("exec: empty command", "exec: コマンドが空です"),
    (
        "SCP needs a connection with the built-in SSH",
        "SCP には内蔵 SSH の接続が必要です",
    ),
    // 構文
    (
        "'then' or a command expected",
        "'then' かコマンドが必要です",
    ),
    (
        "a block cannot follow a one-line if",
        "1 行の if の後にブロックは書けません",
    ),
    ("block is not closed", "ブロックが閉じていません"),
    (
        "break or continue outside a loop",
        "ループの外で break / continue は使えません",
    ),
    ("clause after else", "else の後に節があります"),
    ("command expected", "コマンドが必要です"),
    (
        "do and loop cannot both have a condition",
        "do と loop の両方には条件を書けません",
    ),
    ("else without if", "if の無い else があります"),
    ("expression expected", "式が必要です"),
    ("integer expected", "整数が必要です"),
    ("label name expected", "ラベル名が必要です"),
    ("name expected", "名前が必要です"),
    ("next without for", "for の無い next があります"),
    ("return without call", "call の無い return があります"),
    ("string expected", "文字列が必要です"),
    (
        "unexpected character '{0}'",
        "予期しない文字 '{0}' があります",
    ),
    ("unexpected end of {0}", "{0} が途中で終わっています"),
    (
        "unexpected text at end of line",
        "行末に余分な文字があります",
    ),
    ("unterminated string", "文字列が閉じていません"),
    ("variable expected", "変数が必要です"),
    (
        "bad character code #{0}",
        "文字コード #{0} が正しくありません",
    ),
    (
        "bad character code after #",
        "# の後の文字コードが正しくありません",
    ),
    ("bad number", "数値が正しくありません"),
    // 実行
    ("call is nested too deeply", "call の入れ子が深すぎます"),
    (
        "include is nested too deeply",
        "include の入れ子が深すぎます",
    ),
    ("division by zero", "0 で割りました"),
    ("negative index", "添字が負です"),
    (
        "index {0} out of range for '{1}'",
        "添字 {0} は '{1}' の範囲外です",
    ),
    ("type mismatch for '{0}'", "'{0}' の型が合いません"),
    ("type mismatch", "型が合いません"),
    (
        "this operator needs integers",
        "この演算子には整数が必要です",
    ),
    (
        "'{0}' is already a variable",
        "'{0}' はすでに変数として使われています",
    ),
    ("'{0}' is an array", "'{0}' は配列です"),
    (
        "'{0}' is not an integer variable",
        "'{0}' は整数の変数ではありません",
    ),
    ("unknown array '{0}'", "配列 '{0}' がありません"),
    ("unknown variable '{0}'", "変数 '{0}' がありません"),
    (
        "unsupported command '{0}'",
        "コマンド '{0}' には対応していません",
    ),
    ("no label '{0}'", "ラベル '{0}' がありません"),
    (
        "no host or serial port",
        "ホストかシリアルポートを指定してください",
    ),
    ("bad host '{0}'", "ホスト '{0}' が正しくありません"),
    ("not connected", "接続されていません"),
    // 書式（sprintf）
    ("incomplete format", "書式が途中で終わっています"),
    (
        "not enough arguments for format",
        "書式に対して引数が足りません",
    ),
    ("unsupported format %{0}", "書式 %{0} には対応していません"),
    ("%{0} needs an integer", "%{0} には整数が必要です"),
    // コマンド名が前に付く汎用の型
    (
        "{0}: bad file handle",
        "{0}: ファイルハンドルが正しくありません",
    ),
    ("{0}: array expected", "{0}: 配列が必要です"),
    ("{0}: array name expected", "{0}: 配列名が必要です"),
    ("{0}: bad format", "{0}: 書式が正しくありません"),
    ("{0}: bad size {1}", "{0}: サイズ {1} が正しくありません"),
    (
        "{0}: not a serial connection",
        "{0}: シリアル接続ではありません",
    ),
    ("{0}: wrong number of arguments", "{0}: 引数の数が違います"),
    ("{0}: no string", "{0}: 文字列がありません"),
];

fn compiled() -> &'static [(Regex, &'static str)] {
    static C: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    C.get_or_init(|| {
        let slot = Regex::new(r"\{\d\}").expect("slot");
        TABLE
            .iter()
            .map(|(en, ja)| {
                let mut pattern = String::from("^");
                let mut last = 0;
                for m in slot.find_iter(en) {
                    pattern.push_str(&regex::escape(&en[last..m.start()]));
                    pattern.push_str("(.+?)");
                    last = m.end();
                }
                pattern.push_str(&regex::escape(&en[last..]));
                pattern.push('$');
                (Regex::new(&pattern).expect("pattern"), *ja)
            })
            .collect()
    })
}

/// エラーの文（`ファイル:行:` は付かないもの）を日本語にする。表に無ければ None。
pub fn japanese_message(message: &str) -> Option<String> {
    for (re, ja) in compiled() {
        if let Some(c) = re.captures(message) {
            let mut out = (*ja).to_owned();
            for i in 1..c.len() {
                out = out.replace(&format!("{{{}}}", i - 1), &c[i]);
            }
            return Some(out);
        }
    }
    None
}

/// UI に届くマクロのエラー（[`MacroError`](crate::MacroError) の `ファイル:行: 文`）を日本語にする。
/// `x.ttl の 3 行目: ラベル 'foo' がありません` の形。訳せない文はそのまま返す。
pub fn japanese_error(error: &str) -> String {
    static AT: OnceLock<Regex> = OnceLock::new();
    let at = AT.get_or_init(|| Regex::new(r"^(?s)(.*):(\d+): (.*)$").expect("at"));
    let Some(c) = at.captures(error) else {
        return japanese_message(error).unwrap_or_else(|| error.to_owned());
    };
    let file = std::path::Path::new(&c[1])
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| c[1].to_owned());
    let message = &c[3];
    match japanese_message(message) {
        Some(ja) => format!("{file} の {} 行目: {ja}\n（{message}）", &c[2]),
        None => format!("{file} の {} 行目: {message}", &c[2]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_translated_with_their_values() {
        assert_eq!(
            japanese_message("no label 'loop1'").as_deref(),
            Some("ラベル 'loop1' がありません")
        );
        assert_eq!(
            japanese_message("index 5 out of range for 'arr'").as_deref(),
            Some("添字 5 は 'arr' の範囲外です")
        );
        assert_eq!(
            japanese_message("filereadln: bad file handle").as_deref(),
            Some("filereadln: ファイルハンドルが正しくありません")
        );
        // 具体的な型が汎用の型より先に当たる
        assert_eq!(
            japanese_message("logrotate: count expected").as_deref(),
            Some("logrotate: 世代数が必要です")
        );
        assert_eq!(
            japanese_message("type mismatch").as_deref(),
            Some("型が合いません")
        );
        assert_eq!(japanese_message("connect: Connection refused"), None);
    }

    #[test]
    fn errors_keep_the_place_and_the_original() {
        assert_eq!(
            japanese_error("/Users/a/macros/login.ttl:12: no label 'retry'"),
            "login.ttl の 12 行目: ラベル 'retry' がありません\n（no label 'retry'）"
        );
        // 訳の無い文は場所だけ日本語にする
        assert_eq!(
            japanese_error("x.ttl:3: connect: Connection refused"),
            "x.ttl の 3 行目: connect: Connection refused"
        );
        assert_eq!(japanese_error("something else"), "something else");
    }

    /// インタプリタとパーサが実際に出す文が、表で訳せる
    #[test]
    fn real_interpreter_errors_are_covered() {
        use crate::{Host, Interp};
        struct Null;
        impl Host for Null {
            fn connect(
                &mut self,
                _: &crate::ConnectSpec,
            ) -> Result<std::sync::Arc<mt_core::Session>, String> {
                Err("no".into())
            }
        }
        let cases = [
            "goto nowhere",
            "a = 1 / 0",
            "call nowhere",
            "unknowncmd 1",
            "for i 1 2\n",
            "next",
            "x = 'abc",
        ];
        for src in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("t.ttl");
            std::fs::write(&path, src).unwrap();
            let mut host = Null;
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let err = Interp::new(&mut host, None, stop)
                .run_file(&path, &[])
                .expect_err(src)
                .to_string();
            let ja = japanese_error(&err);
            assert!(ja.contains("（"), "{src}: {err} → {ja}");
        }
    }
}
