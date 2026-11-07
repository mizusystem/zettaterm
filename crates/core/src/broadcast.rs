//! コマンドのブロードキャスト（Tera Term の「コントロール > ブロードキャストコマンド」）の判断。
//! 送り先の選び方や画面は UI が持ち、ここは履歴とリアルタイム送信の差分だけを扱う。

/// 履歴の数の既定（Tera Term の MaxBroadcatHistory の既定）
pub const DEFAULT_HISTORY: usize = 99;

/// 送ったコマンドを履歴の先頭に足す。同じものは前のほうを消し、`max` 件までにする。空は足さない。
pub fn push_history(history: &mut Vec<String>, command: &str, max: usize) {
    if command.is_empty() {
        return;
    }
    history.retain(|c| c != command);
    history.insert(0, command.to_owned());
    history.truncate(max.max(1));
}

/// リアルタイム送信：入力欄が `old` から `new` に変わったとき、送るもの。
/// （消す文字の数, 足す文字列）。共通の先頭より後ろを消して打ち直す（文字単位）。
pub fn realtime_diff(old: &str, new: &str) -> (usize, String) {
    let common = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let erase = old.chars().count() - common;
    let insert: String = new.chars().skip(common).collect();
    (erase, insert)
}

/// Enter キーで送る改行（Tera Term の「Enter キー」の CRLF / CR / LF）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BroadcastNewline {
    /// 送り先の接続の「送信」の改行コード
    #[default]
    Session,
    Cr,
    Lf,
    CrLf,
}

impl BroadcastNewline {
    /// 送るバイト。`Session` は None（送り先の Enter キーとして送る）。
    pub fn bytes(self) -> Option<&'static [u8]> {
        match self {
            Self::Session => None,
            Self::Cr => Some(b"\r"),
            Self::Lf => Some(b"\n"),
            Self::CrLf => Some(b"\r\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_newest_first_without_duplicates() {
        let mut h = vec![];
        for c in ["show ver", "show clock", "show ver", ""] {
            push_history(&mut h, c, 2);
        }
        assert_eq!(h, ["show ver", "show clock"]);
        push_history(&mut h, "exit", 2);
        assert_eq!(h, ["exit", "show ver"]);
    }

    #[test]
    fn realtime_sends_only_the_change() {
        assert_eq!(realtime_diff("sho", "show"), (0, "w".into()));
        assert_eq!(realtime_diff("show", "sho"), (1, String::new()));
        assert_eq!(realtime_diff("日本", "日米語"), (1, "米語".into()));
        assert_eq!(realtime_diff("", ""), (0, String::new()));
    }
}
