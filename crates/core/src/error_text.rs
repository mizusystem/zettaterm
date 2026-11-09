//! エラーの表示のための分類。
//!
//! コアや OS のエラーは英語の文（`Connection refused (os error 61)` など）で UI に届く。
//! UI はここで種類を見分け、Tera Term の文言（`MSG_COMM_REFUSE_ERROR` など）か自分の文言で出す。
//! 元の文は詳しい情報として添える。macOS と Linux で OS の文が違うものは両方を見る。

/// エラーの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    /// 接続を拒否された（Tera Term の MSG_COMM_REFUSE_ERROR）
    Refused,
    /// 接続がタイムアウトした（MSG_COMM_TIMEOUT_ERROR）
    TimedOut,
    /// ネットワークやホストに届かない（MSG_COMM_REACH_ERROR）
    Unreachable,
    /// ホスト名を引けない（MSG_INVALID_HOST_ERROR）
    HostNotFound,
    /// ファイルやポートが無い（MSG_CANTOPEN_ERROR_NOTFOUND）
    NotFound,
    /// 開けない。権限が無いか、ほかで使っている（MSG_CANTOPEN_ERROR_DENIED。Tera Term も使用中の COM はこれ）
    Denied,
    /// 相手が接続を切った（接続中のリセットや切れたパイプ）
    ConnectionLost,
    /// SSH の認証を中止した
    AuthCanceled,
    /// SSH のホスト鍵を受け入れなかった
    HostKeyRejected,
    /// ホストが空
    HostEmpty,
    /// この OS では 8 ビット + space パリティを使えない
    SpaceParityUnsupported,
    /// この OS では DSR/DTR フロー制御を使えない
    DsrDtrUnsupported,
    /// ファイル転送の最中
    TransferRunning,
    /// 見分けられない（元の文をそのまま出す）
    Other,
}

/// 分類した結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorInfo {
    pub class: ErrorClass,
    /// 対象（`/dev/ttyUSB0: No such file ...` の `/dev/ttyUSB0`）。文に無ければ None
    pub subject: Option<String>,
    /// 元の文
    pub detail: String,
}

/// 英語のエラーの文を分類する。
pub fn classify(message: &str) -> ErrorInfo {
    let m = message.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    let class = if has(&["authentication canceled"]) {
        ErrorClass::AuthCanceled
    } else if has(&["unknown server key"]) {
        ErrorClass::HostKeyRejected
    } else if m.trim() == "host is empty" {
        ErrorClass::HostEmpty
    } else if has(&["8-bit data with space parity"]) {
        ErrorClass::SpaceParityUnsupported
    } else if has(&["dsr/dtr flow control"]) {
        ErrorClass::DsrDtrUnsupported
    } else if has(&["a transfer is already running"]) {
        ErrorClass::TransferRunning
    } else if has(&["connection refused", "(os error 61)", "(os error 111)"]) {
        ErrorClass::Refused
    } else if has(&["timed out", "(os error 60)", "(os error 110)"]) {
        ErrorClass::TimedOut
    } else if has(&[
        "no route to host",
        "network is unreachable",
        "network is down",
        "host is down",
        "host is unreachable",
        // OS の文は言語で変わるので番号でも見る（macOS: 50, 51, 64, 65 / Linux: 100, 101, 112, 113）
        "(os error 50)",
        "(os error 51)",
        "(os error 64)",
        "(os error 65)",
        "(os error 100)",
        "(os error 101)",
        "(os error 112)",
        "(os error 113)",
    ]) {
        ErrorClass::Unreachable
    } else if has(&[
        "failed to lookup address",
        "nodename nor servname",
        "name or service not known",
        "temporary failure in name resolution",
        "no address associated with hostname",
        ": no address",
    ]) {
        ErrorClass::HostNotFound
    } else if has(&["no such file or directory", "(os error 2)"]) {
        ErrorClass::NotFound
    } else if has(&[
        "permission denied",
        "access denied",
        "resource busy",
        "(os error 13)",
        "(os error 16)",
    ]) {
        ErrorClass::Denied
    } else if has(&[
        "connection reset",
        "broken pipe",
        "connection aborted",
        // macOS: 53, 54 / Linux: 103, 104 / 両方: 32
        "(os error 32)",
        "(os error 53)",
        "(os error 54)",
        "(os error 103)",
        "(os error 104)",
    ]) {
        ErrorClass::ConnectionLost
    } else {
        ErrorClass::Other
    };
    ErrorInfo {
        class,
        subject: subject(message),
        detail: message.to_owned(),
    }
}

/// 先頭の `対象: ` を取り出す（空白を含まない語だけ。`/dev/cu.usbserial-210: ...` や `host: no address`）。
fn subject(message: &str) -> Option<String> {
    let (head, _) = message.split_once(": ")?;
    (!head.is_empty() && !head.contains(char::is_whitespace)).then(|| head.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_messages_on_macos_and_linux() {
        let c = |s: &str| classify(s).class;
        assert_eq!(c("Connection refused (os error 61)"), ErrorClass::Refused);
        assert_eq!(c("Connection refused (os error 111)"), ErrorClass::Refused);
        assert_eq!(c("Operation timed out (os error 60)"), ErrorClass::TimedOut);
        assert_eq!(c("connection timed out"), ErrorClass::TimedOut);
        assert_eq!(c("No route to host (os error 65)"), ErrorClass::Unreachable);
        assert_eq!(
            c("Network is unreachable (os error 101)"),
            ErrorClass::Unreachable
        );
        assert_eq!(
            c("failed to lookup address information: nodename nor servname provided, or not known"),
            ErrorClass::HostNotFound
        );
        assert_eq!(
            c("failed to lookup address information: Name or service not known"),
            ErrorClass::HostNotFound
        );
        assert_eq!(c("nohost.example: no address"), ErrorClass::HostNotFound);
        assert_eq!(c("No such file or directory"), ErrorClass::NotFound);
        assert_eq!(c("Permission denied (os error 13)"), ErrorClass::Denied);
        assert_eq!(c("Resource busy"), ErrorClass::Denied);
        assert_eq!(
            c("Device or resource busy (os error 16)"),
            ErrorClass::Denied
        );
        assert_eq!(
            c("Connection reset by peer (os error 54)"),
            ErrorClass::ConnectionLost
        );
        // 日本語の Linux では OS の文が日本語になる。番号で見分ける
        assert_eq!(
            c("接続を拒否されました (os error 111)"),
            ErrorClass::Refused
        );
        assert_eq!(
            c("ホストへの経路がありません (os error 113)"),
            ErrorClass::Unreachable
        );
        assert_eq!(
            c("デバイスもしくはリソースがビジー状態です (os error 16)"),
            ErrorClass::Denied
        );
        assert_eq!(c("authentication canceled"), ErrorClass::AuthCanceled);
        assert_eq!(c("Unknown server key"), ErrorClass::HostKeyRejected);
        assert_eq!(c("host is empty"), ErrorClass::HostEmpty);
        assert_eq!(
            c("8-bit data with space parity is not supported on this OS"),
            ErrorClass::SpaceParityUnsupported
        );
        assert_eq!(
            c("a transfer is already running"),
            ErrorClass::TransferRunning
        );
        assert_eq!(c("line 3: unknown command 'foo'"), ErrorClass::Other);
    }

    #[test]
    fn subject_is_the_leading_word() {
        let i = classify("/dev/cu.usbserial-210: No such file or directory (os error 2)");
        assert_eq!(i.class, ErrorClass::NotFound);
        assert_eq!(i.subject.as_deref(), Some("/dev/cu.usbserial-210"));
        assert_eq!(classify("Permission denied (os error 13)").subject, None);
        // 空白を含む前置きは対象にしない
        assert_eq!(
            classify("failed to lookup address information: x").subject,
            None
        );
    }
}
