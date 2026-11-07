//! 相手から届いた制御シーケンスのうち、設定で断るものを VT に渡す前に取り除く。
//!
//! - スクロールバッファの消去 `CSI 3 J`（Tera Term の ClearScrollBufferFromRemote）
//! - カーソルの形 `CSI Ps SP q`（DECSCUSR）と点滅 `CSI ? 12 h/l`（CursorCtrlSequence）
//!
//! 受信の塊をまたいだシーケンスも、途中から続けて見る。

use std::borrow::Cow;

/// 受け付けるか。既定は Tera Term と同じく、どちらも断る。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteControl {
    /// `CSI 3 J` でスクロールバッファを消すのを受け付ける
    pub clear_scrollback: bool,
    /// カーソルの形と点滅を変えるのを受け付ける
    pub cursor_style: bool,
}

/// CSI の長さの上限（これより長いものは調べずにそのまま渡す）
const MAX_CSI: usize = 64;

#[derive(Default)]
pub(crate) struct SeqFilter {
    pub(crate) allow: RemoteControl,
    /// ESC か CSI の途中まで（決まるまで持っておく）
    pending: Vec<u8>,
}

impl SeqFilter {
    pub(crate) fn filter<'a>(&mut self, input: &'a [u8]) -> Cow<'a, [u8]> {
        if self.allow.clear_scrollback && self.allow.cursor_style && self.pending.is_empty() {
            return Cow::Borrowed(input);
        }
        if self.pending.is_empty() && !input.contains(&0x1b) {
            return Cow::Borrowed(input);
        }
        let mut out = Vec::with_capacity(input.len() + self.pending.len());
        for &b in input {
            if self.pending.is_empty() {
                if b == 0x1b {
                    self.pending.push(b);
                } else {
                    out.push(b);
                }
                continue;
            }
            self.pending.push(b);
            if self.pending.len() == 2 {
                if b != b'[' {
                    // CSI ではない。ESC ESC なら次の ESC から見直す
                    let last = self.pending.pop();
                    out.append(&mut self.pending);
                    if last == Some(0x1b) {
                        self.pending.push(0x1b);
                    } else if let Some(l) = last {
                        out.push(l);
                    }
                }
                continue;
            }
            // CSI の終わり（0x40〜0x7e）まで集める
            if (0x40..=0x7e).contains(&b) {
                let seq = std::mem::take(&mut self.pending);
                out.extend(self.rewrite(&seq));
            } else if self.pending.len() > MAX_CSI || !(0x20..=0x3f).contains(&b) {
                // CSI として読めない・長すぎる。そのまま渡す
                out.append(&mut self.pending);
            }
        }
        Cow::Owned(out)
    }

    /// 1 つの CSI（ESC [ から終わりの文字まで）。断るものは除くか書き換える。
    fn rewrite(&self, seq: &[u8]) -> Vec<u8> {
        let body = &seq[2..seq.len() - 1];
        let fin = seq[seq.len() - 1];
        if !self.allow.clear_scrollback && fin == b'J' && body == b"3" {
            return Vec::new();
        }
        if !self.allow.cursor_style {
            if fin == b'q' && body.last() == Some(&b' ') {
                return Vec::new();
            }
            if (fin == b'h' || fin == b'l') && body.first() == Some(&b'?') {
                let params: Vec<&[u8]> = body[1..].split(|&c| c == b';').collect();
                if params.contains(&&b"12"[..]) {
                    let kept: Vec<&[u8]> = params.into_iter().filter(|p| *p != b"12").collect();
                    if kept.is_empty() {
                        return Vec::new();
                    }
                    let mut out = b"\x1b[?".to_vec();
                    out.extend(kept.join(&b';'));
                    out.push(fin);
                    return out;
                }
            }
        }
        seq.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut SeqFilter, parts: &[&[u8]]) -> Vec<u8> {
        parts
            .iter()
            .flat_map(|p| f.filter(p).into_owned())
            .collect()
    }

    #[test]
    fn refused_sequences_are_removed() {
        let mut f = SeqFilter::default();
        assert_eq!(run(&mut f, &[b"a\x1b[3Jb\x1b[2Jc"]), b"ab\x1b[2Jc");
        assert_eq!(
            run(&mut f, &[b"\x1b[5 qx\x1b[?12h\x1b[?12;25l"]),
            b"x\x1b[?25l"
        );
        // 塊をまたいでも
        assert_eq!(run(&mut f, &[b"x\x1b", b"[", b"3", b"Jy"]), b"xy");
        // ほかのシーケンスと ESC だけのものはそのまま
        assert_eq!(
            run(&mut f, &[b"\x1b[1;31m\x1b7\x1b\x1b[3J!"]),
            b"\x1b[1;31m\x1b7\x1b!"
        );
    }

    #[test]
    fn allowed_sequences_pass() {
        let mut f = SeqFilter {
            allow: RemoteControl {
                clear_scrollback: true,
                cursor_style: true,
            },
            ..Default::default()
        };
        assert_eq!(run(&mut f, &[b"\x1b[3J\x1b[5 q"]), b"\x1b[3J\x1b[5 q");
    }
}
