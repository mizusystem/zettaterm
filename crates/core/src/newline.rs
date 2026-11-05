//! 改行コードの変換（Tera Term の「設定 > 端末 > 改行コード」）。
//!
//! 受信（`CRReceive`）は Tera Term の vtterm.c の ProcessCR / ProcessLF と同じ規則で変換する。
//! - CR：そのまま
//! - CR+LF：受信した CR を CR+LF として扱う（CR だけを送ってくる相手向け）
//! - LF：受信した LF を CR+LF として扱う（LF だけを送ってくる相手向け。ESP32 など）
//! - AUTO：CR と LF のどちらも CR+LF として扱い、直後に来た反対側の 1 文字は捨てる
//!
//! 送信（`CRSend`）は Enter キーと貼り付けの改行を CR / CR+LF / LF のどれで送るか。

const CR: u8 = b'\r';
const LF: u8 = b'\n';

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NewlineReceive {
    #[default]
    Cr,
    CrLf,
    Lf,
    Auto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NewlineSend {
    #[default]
    Cr,
    CrLf,
    Lf,
}

impl NewlineSend {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            NewlineSend::Cr => b"\r",
            NewlineSend::CrLf => b"\r\n",
            NewlineSend::Lf => b"\n",
        }
    }

    /// 送信データ中の CR を、この改行コードに置き換える。
    pub fn apply(self, bytes: Vec<u8>) -> Vec<u8> {
        if self == NewlineSend::Cr || !bytes.contains(&CR) {
            return bytes;
        }
        let mut out = Vec::with_capacity(bytes.len() + 8);
        for b in bytes {
            if b == CR {
                out.extend_from_slice(self.bytes());
            } else {
                out.push(b);
            }
        }
        out
    }
}

/// 受信側の変換器。チャンクの境目をまたいで状態を持つ。
#[derive(Debug, Default)]
pub struct ReceiveConverter {
    mode: NewlineReceive,
    prev: u8,
    /// 直前の CR / LF から CR+LF を作った（AUTO 用）
    prev_generated: bool,
}

impl ReceiveConverter {
    pub fn new(mode: NewlineReceive) -> Self {
        Self {
            mode,
            prev: 0,
            prev_generated: false,
        }
    }

    pub fn mode(&self) -> NewlineReceive {
        self.mode
    }

    pub fn set_mode(&mut self, mode: NewlineReceive) {
        *self = Self::new(mode);
    }

    /// 変換が要らなければ入力をそのまま返す。
    pub fn convert<'a>(&mut self, bytes: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        if self.mode == NewlineReceive::Cr {
            return std::borrow::Cow::Borrowed(bytes);
        }
        let mut out = Vec::with_capacity(bytes.len() + 16);
        for &b in bytes {
            match (self.mode, b) {
                (NewlineReceive::CrLf, CR) => out.extend_from_slice(b"\r\n"),
                (NewlineReceive::Lf, LF) => out.extend_from_slice(b"\r\n"),
                (NewlineReceive::Auto, CR | LF) => {
                    let opposite = if b == CR { LF } else { CR };
                    if self.prev == opposite && self.prev_generated {
                        // CR+LF（または LF+CR）の 2 文字目。1 回分の改行はもう出している
                        self.prev_generated = false;
                    } else {
                        out.extend_from_slice(b"\r\n");
                        self.prev_generated = true;
                    }
                }
                _ => out.push(b),
            }
            self.prev = b;
        }
        std::borrow::Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(mode: NewlineReceive, chunks: &[&[u8]]) -> Vec<u8> {
        let mut c = ReceiveConverter::new(mode);
        chunks
            .iter()
            .flat_map(|ch| c.convert(ch).into_owned())
            .collect()
    }

    #[test]
    fn cr_mode_passes_through() {
        assert_eq!(conv(NewlineReceive::Cr, &[b"a\nb\r\nc\r"]), b"a\nb\r\nc\r");
    }

    #[test]
    fn crlf_mode_expands_cr() {
        assert_eq!(conv(NewlineReceive::CrLf, &[b"a\rb\n"]), b"a\r\nb\n");
    }

    #[test]
    fn lf_mode_expands_lf() {
        assert_eq!(conv(NewlineReceive::Lf, &[b"a\nb\rc"]), b"a\r\nb\rc");
    }

    #[test]
    fn auto_mode_treats_any_newline_as_one_line_break() {
        assert_eq!(
            conv(NewlineReceive::Auto, &[b"a\nb\rc\r\nd\n\re"]),
            b"a\r\nb\r\nc\r\nd\r\ne"
        );
        // 空行は 2 回の改行として残る
        assert_eq!(conv(NewlineReceive::Auto, &[b"a\r\n\r\nb"]), b"a\r\n\r\nb");
        assert_eq!(conv(NewlineReceive::Auto, &[b"a\n\nb"]), b"a\r\n\r\nb");
    }

    #[test]
    fn auto_mode_keeps_state_across_chunks() {
        assert_eq!(conv(NewlineReceive::Auto, &[b"a\r", b"\nb"]), b"a\r\nb");
    }

    #[test]
    fn send_newline() {
        assert_eq!(NewlineSend::Cr.apply(b"a\rb".to_vec()), b"a\rb");
        assert_eq!(NewlineSend::CrLf.apply(b"a\rb".to_vec()), b"a\r\nb");
        assert_eq!(NewlineSend::Lf.apply(b"a\rb\r".to_vec()), b"a\nb\n");
    }
}
