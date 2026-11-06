//! 貼り付けの確認（Tera Term の ConfirmChangePaste と ConfirmChangePasteStringFile）。
//!
//! 改行を含む文字列や、危険な語（`reload` や `rm -rf` など）を含む文字列を貼り付ける前に、
//! UI が確認ダイアログを出すかどうかをここで決める。

/// 既定の危険な語。ネットワーク機器とサーバーで、誤って流すと止まったり消えたりするもの。
pub const DEFAULT_DANGEROUS_WORDS: &[&str] = &[
    "reload",
    "reboot",
    "shutdown",
    "halt",
    "poweroff",
    "write erase",
    "erase",
    "format",
    "delete",
    "rm -rf",
    "rm -r",
    "mkfs",
    "dd if=",
    "init 0",
    "init 6",
    "factory-reset",
    "request system reboot",
    "request system zeroize",
];

/// 貼り付けようとしている文字列の中身。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasteCheck {
    /// 行数（末尾の改行は数えない）
    pub lines: usize,
    /// 改行を含む（貼り付けるとそのまま実行される）
    pub multiline: bool,
    /// 含まれていた危険な語（`words` に書かれた形、重複なし）
    pub dangerous: Vec<String>,
}

impl PasteCheck {
    /// 確認ダイアログを出すべきか。`confirm_multiline` は「改行を含むときも確認する」。
    pub fn needs_confirmation(&self, confirm_multiline: bool) -> bool {
        !self.dangerous.is_empty() || (confirm_multiline && self.multiline)
    }
}

/// 語の前後が英数字でないか（`reloaded` や `undelete` を拾わない）。
fn at_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    edge(before) && edge(after)
}

/// `text` を調べる。`words` は危険な語（大文字小文字は区別しない、空の行は無視）。
pub fn check_paste(text: &str, words: &[String]) -> PasteCheck {
    let body = text.trim_end_matches(['\r', '\n']);
    let multiline = body.contains(['\r', '\n']) || body.len() != text.len();
    let lines = if body.is_empty() {
        0
    } else {
        body.split(['\n']).count()
    };
    let lower = text.to_lowercase();
    let mut dangerous: Vec<String> = vec![];
    for w in words {
        let w = w.trim();
        if w.is_empty() || dangerous.iter().any(|d| d == w) {
            continue;
        }
        let needle = w.to_lowercase();
        let hit = lower
            .match_indices(&needle)
            .any(|(i, m)| at_word_boundary(&lower, i, i + m.len()));
        if hit {
            dangerous.push(w.to_owned());
        }
    }
    PasteCheck {
        lines,
        multiline,
        dangerous,
    }
}

/// 既定の危険な語の一覧（設定の初期値）。
pub fn default_dangerous_words() -> Vec<String> {
    DEFAULT_DANGEROUS_WORDS
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

/// 末尾の改行を取り除く（TrimTrailingNLonPaste）。
pub fn trim_trailing_newlines(text: &str) -> &str {
    text.trim_end_matches(['\r', '\n'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<String> {
        default_dangerous_words()
    }

    #[test]
    fn single_line_without_dangerous_words_passes() {
        let c = check_paste("show ip int brief", &words());
        assert_eq!(c.lines, 1);
        assert!(!c.multiline);
        assert!(!c.needs_confirmation(true));
    }

    #[test]
    fn multiline_and_trailing_newline_count_as_multiline() {
        let c = check_paste("conf t\ninterface gi0/1\n", &words());
        assert_eq!(c.lines, 2);
        assert!(c.multiline);
        assert!(c.needs_confirmation(true));
        assert!(!c.needs_confirmation(false));
        // 改行 1 つで終わるだけでも、貼った瞬間に実行される
        assert!(check_paste("show run\r\n", &words()).multiline);
    }

    #[test]
    fn dangerous_words_are_found_as_whole_words_ignoring_case() {
        let c = check_paste("RELOAD in 5\nwrite erase", &words());
        assert_eq!(c.dangerous, ["reload", "write erase", "erase"]);
        assert!(c.needs_confirmation(false));
        // 語の一部は拾わない
        assert!(
            check_paste("show reloaded-count; undelete", &words())
                .dangerous
                .is_empty()
        );
        assert_eq!(
            check_paste("sudo rm -rf /tmp/x", &words()).dangerous,
            ["rm -rf"]
        );
    }

    #[test]
    fn custom_words_and_trimming() {
        let custom = vec!["clear counters".to_owned(), "  ".to_owned()];
        assert_eq!(
            check_paste("Clear Counters", &custom).dangerous,
            ["clear counters"]
        );
        assert_eq!(trim_trailing_newlines("abc\r\n\n"), "abc");
        assert_eq!(check_paste("", &custom).lines, 0);
    }
}
