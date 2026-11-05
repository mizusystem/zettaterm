//! 文字コードの変換（Tera Term の「エンコーディング-受信 / 送信」）。
//!
//! 端末の中（VT の解釈と画面）はすべて UTF-8 で扱い、相手との間だけ変換する。
//! - 受信：相手の文字コード → UTF-8（チャンクの境目で文字が割れても続けて変換できる）
//! - 送信：キー入力と貼り付けの UTF-8 → 相手の文字コード。ASCII（エスケープシーケンスを含む）はそのまま
//!
//! JIS（ISO-2022-JP）は、文字集合の切り替え（ESC $ B など）だけを読み、それ以外のエスケープシーケンスは
//! 端末の制御として VT に渡す。

use std::borrow::Cow;

use encoding_rs::{Decoder, EUC_JP, Encoding, SHIFT_JIS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Charset {
    #[default]
    Utf8,
    ShiftJis,
    EucJp,
    /// ISO-2022-JP
    Jis,
}

/// 受信側の変換器。
pub struct ReceiveDecoder {
    charset: Charset,
    inner: Inner,
}

enum Inner {
    Utf8,
    Legacy(Decoder),
    Jis(JisDecoder),
}

impl ReceiveDecoder {
    pub fn new(charset: Charset) -> Self {
        let inner = match charset {
            Charset::Utf8 => Inner::Utf8,
            Charset::ShiftJis => Inner::Legacy(SHIFT_JIS.new_decoder_without_bom_handling()),
            Charset::EucJp => Inner::Legacy(EUC_JP.new_decoder_without_bom_handling()),
            Charset::Jis => Inner::Jis(JisDecoder::default()),
        };
        Self { charset, inner }
    }

    pub fn charset(&self) -> Charset {
        self.charset
    }

    /// 受信バイトを UTF-8 にする。UTF-8 ならそのまま返す。
    pub fn decode<'a>(&mut self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        match &mut self.inner {
            Inner::Utf8 => Cow::Borrowed(bytes),
            Inner::Legacy(d) => Cow::Owned(decode_legacy(d, bytes).into_bytes()),
            Inner::Jis(d) => Cow::Owned(d.decode(bytes).into_bytes()),
        }
    }
}

fn decode_legacy(d: &mut Decoder, bytes: &[u8]) -> String {
    let mut out = String::with_capacity(
        d.max_utf8_buffer_length(bytes.len())
            .unwrap_or(bytes.len() * 3),
    );
    let _ = d.decode_to_string(bytes, &mut out, false);
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum JisSet {
    #[default]
    Ascii,
    /// JIS X 0201 カタカナ（半角カナ）
    Kana,
    /// JIS X 0208 漢字
    Kanji,
}

#[derive(Default)]
struct JisDecoder {
    set: JisSet,
    /// 読みかけのエスケープシーケンス
    esc: Vec<u8>,
    /// 漢字の 1 バイト目
    lead: Option<u8>,
}

impl JisDecoder {
    fn decode(&mut self, bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            if !self.esc.is_empty() {
                self.esc.push(b);
                match self.esc.as_slice() {
                    [0x1b, b'$'] | [0x1b, b'('] => continue,
                    [0x1b, b'$', b'@'] | [0x1b, b'$', b'B'] => self.switch(JisSet::Kanji),
                    [0x1b, b'(', b'B'] | [0x1b, b'(', b'J'] => self.switch(JisSet::Ascii),
                    [0x1b, b'(', b'I'] => self.switch(JisSet::Kana),
                    // 文字集合の切り替えではない。端末の制御として VT に渡す
                    _ => out.push_str(&String::from_utf8_lossy(&std::mem::take(&mut self.esc))),
                }
                self.esc.clear();
                continue;
            }
            match b {
                0x1b => {
                    self.lead = None;
                    self.esc.push(b);
                }
                0x0e => self.set = JisSet::Kana,  // SO
                0x0f => self.set = JisSet::Ascii, // SI
                0x00..=0x20 | 0x7f => {
                    self.lead = None;
                    out.push(b as char);
                }
                0x21..=0x7e => match self.set {
                    JisSet::Ascii => out.push(b as char),
                    JisSet::Kana if b <= 0x5f => {
                        out.push(char::from_u32(0xff61 + u32::from(b - 0x21)).unwrap_or('?'));
                    }
                    JisSet::Kana => out.push('\u{fffd}'),
                    JisSet::Kanji => match self.lead.take() {
                        None => self.lead = Some(b),
                        Some(lead) => {
                            // JIS X 0208 の 2 バイトに 0x80 を足すと EUC-JP になる
                            let euc = [lead | 0x80, b | 0x80];
                            let (s, _) = EUC_JP.decode_without_bom_handling(&euc);
                            out.push_str(&s);
                        }
                    },
                },
                _ => out.push('\u{fffd}'),
            }
        }
        out
    }

    fn switch(&mut self, set: JisSet) {
        self.set = set;
        self.lead = None;
    }
}

/// 送信データ（UTF-8）を相手の文字コードにする。ASCII はそのまま、表せない文字は `?` にする。
pub fn encode_send(charset: Charset, bytes: Vec<u8>) -> Vec<u8> {
    let encoding: &'static Encoding = match charset {
        Charset::Utf8 => return bytes,
        Charset::ShiftJis => SHIFT_JIS,
        Charset::EucJp => EUC_JP,
        Charset::Jis => return encode_jis(&String::from_utf8_lossy(&bytes)),
    };
    if bytes.is_ascii() {
        return bytes;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut out = Vec::with_capacity(bytes.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch as u8);
            continue;
        }
        let mut buf = [0u8; 4];
        let (encoded, _, had_errors) = encoding.encode(ch.encode_utf8(&mut buf));
        if had_errors {
            out.push(b'?');
        } else {
            out.extend_from_slice(&encoded);
        }
    }
    out
}

/// JIS（ISO-2022-JP）。漢字は ESC $ B、半角カナは ESC ( I で切り替え、ASCII に戻すときは ESC ( B。
fn encode_jis(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    let mut set = JisSet::Ascii;
    let mut switch = |out: &mut Vec<u8>, to: JisSet| {
        if set != to {
            out.extend_from_slice(match to {
                JisSet::Ascii => b"\x1b(B",
                JisSet::Kana => b"\x1b(I",
                JisSet::Kanji => b"\x1b$B",
            });
            set = to;
        }
    };
    for ch in text.chars() {
        let c = ch as u32;
        if ch.is_ascii() {
            switch(&mut out, JisSet::Ascii);
            out.push(ch as u8);
        } else if (0xff61..=0xff9f).contains(&c) {
            switch(&mut out, JisSet::Kana);
            out.push((c - 0xff61 + 0x21) as u8);
        } else {
            let mut buf = [0u8; 4];
            let (euc, _, had_errors) = EUC_JP.encode(ch.encode_utf8(&mut buf));
            match (&euc[..], had_errors) {
                ([a @ 0xa1..=0xfe, b @ 0xa1..=0xfe], false) => {
                    switch(&mut out, JisSet::Kanji);
                    out.push(a & 0x7f);
                    out.push(b & 0x7f);
                }
                _ => {
                    switch(&mut out, JisSet::Ascii);
                    out.push(b'?');
                }
            }
        }
    }
    switch(&mut out, JisSet::Ascii);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(charset: Charset, chunks: &[&[u8]]) -> String {
        let mut d = ReceiveDecoder::new(charset);
        let mut out = Vec::new();
        for c in chunks {
            out.extend_from_slice(&d.decode(c));
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn utf8_passes_through() {
        assert_eq!(decode_all(Charset::Utf8, &["日本語".as_bytes()]), "日本語");
    }

    #[test]
    fn shift_jis_and_euc_jp_across_chunk_boundaries() {
        let (sjis, _, _) = SHIFT_JIS.encode("漢字ｶﾅ\x1b[1mA");
        let (a, b) = sjis.split_at(1); // 「漢」の 1 バイト目で割る
        assert_eq!(decode_all(Charset::ShiftJis, &[a, b]), "漢字ｶﾅ\x1b[1mA");
        let (euc, _, _) = EUC_JP.encode("表示ﾃｽﾄ\r\n");
        let (a, b) = euc.split_at(3);
        assert_eq!(decode_all(Charset::EucJp, &[a, b]), "表示ﾃｽﾄ\r\n");
    }

    #[test]
    fn jis_switches_charsets_and_keeps_terminal_escapes() {
        // 「日本」半角カナ「ｱ」、途中に色のエスケープシーケンス
        let input = b"A\x1b$BF|K\\\x1b(B\x1b[31mB\x1b(I1\x1b(Bz";
        assert_eq!(decode_all(Charset::Jis, &[input]), "A日本\x1b[31mBｱz");
        // エスケープシーケンスの途中で割れても読める
        assert_eq!(
            decode_all(Charset::Jis, &[b"\x1b$", b"BF|\x1b(", b"B!"]),
            "日!"
        );
    }

    #[test]
    fn encode_for_sending() {
        assert_eq!(
            encode_send(Charset::Utf8, "あ".as_bytes().to_vec()),
            "あ".as_bytes()
        );
        assert_eq!(
            encode_send(Charset::ShiftJis, b"ls -l\r".to_vec()),
            b"ls -l\r"
        );
        assert_eq!(
            encode_send(Charset::ShiftJis, "aあ".as_bytes().to_vec()),
            b"a\x82\xa0"
        );
        assert_eq!(
            encode_send(Charset::EucJp, "あ".as_bytes().to_vec()),
            b"\xa4\xa2"
        );
        // 表せない文字は ?
        assert_eq!(
            encode_send(Charset::ShiftJis, "😀".as_bytes().to_vec()),
            b"?"
        );
        // JIS は連なりごとに囲む。半角カナは ESC ( I
        assert_eq!(
            encode_send(Charset::Jis, "日本a".as_bytes().to_vec()),
            b"\x1b$BF|K\\\x1b(Ba"
        );
        assert_eq!(
            encode_send(Charset::Jis, "ｱ".as_bytes().to_vec()),
            b"\x1b(I1\x1b(B"
        );
        assert_eq!(encode_send(Charset::Jis, b"ls\r".to_vec()), b"ls\r");
    }

    #[test]
    fn round_trip_through_each_charset() {
        let text = "漢字とｶﾀｶﾅ and ASCII";
        for cs in [Charset::ShiftJis, Charset::EucJp, Charset::Jis] {
            let encoded = encode_send(cs, text.as_bytes().to_vec());
            assert_eq!(decode_all(cs, &[&encoded]), text, "{cs:?}");
        }
    }
}
