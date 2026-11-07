//! 文字コードの変換（Tera Term の「エンコーディング-受信 / 送信」）。
//!
//! 端末の中（VT の解釈と画面）はすべて UTF-8 で扱い、相手との間だけ変換する。
//! - 受信：相手の文字コード → UTF-8（チャンクの境目で文字が割れても続けて変換できる）
//! - 送信：キー入力と貼り付けの UTF-8 → 相手の文字コード。ASCII（エスケープシーケンスを含む）はそのまま
//!
//! JIS（ISO-2022-JP）は、文字集合の切り替え（ESC $ B など）だけを読み、それ以外のエスケープシーケンスは
//! 端末の制御として VT に渡す。

use std::borrow::Cow;

use encoding_rs::{
    BIG5, Decoder, EUC_JP, EUC_KR, Encoding, GBK, IBM866, ISO_8859_2, ISO_8859_15, KOI8_R,
    SHIFT_JIS, WINDOWS_1251, WINDOWS_1252,
};
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
    /// 韓国語（KS X 1001 / CP949。Tera Term の KS5601）
    EucKr,
    /// 簡体字中国語（GB2312 / CP936）
    Gbk,
    /// 繁体字中国語（CP950）
    Big5,
    /// ロシア語
    Koi8R,
    Windows1251,
    /// CP866
    Ibm866,
    /// 西欧（ISO-8859-1 の上位互換）
    Windows1252,
    /// 中欧
    Iso8859_2,
    /// 西欧（ユーロ記号入り）
    Iso8859_15,
}

impl Charset {
    /// 選べる文字コード（メニューの並び）。
    pub const ALL: [Charset; 13] = [
        Charset::Utf8,
        Charset::ShiftJis,
        Charset::EucJp,
        Charset::Jis,
        Charset::EucKr,
        Charset::Gbk,
        Charset::Big5,
        Charset::Koi8R,
        Charset::Windows1251,
        Charset::Ibm866,
        Charset::Windows1252,
        Charset::Iso8859_2,
        Charset::Iso8859_15,
    ];

    /// メニューに出す名前。
    pub fn label(self) -> &'static str {
        match self {
            Charset::Utf8 => "UTF-8",
            Charset::ShiftJis => "Shift_JIS",
            Charset::EucJp => "EUC-JP",
            Charset::Jis => "JIS",
            Charset::EucKr => "EUC-KR (KS5601)",
            Charset::Gbk => "GBK (GB2312)",
            Charset::Big5 => "Big5",
            Charset::Koi8R => "KOI8-R",
            Charset::Windows1251 => "Windows-1251",
            Charset::Ibm866 => "CP866",
            Charset::Windows1252 => "Windows-1252 (Latin-1)",
            Charset::Iso8859_2 => "ISO-8859-2",
            Charset::Iso8859_15 => "ISO-8859-15",
        }
    }

    /// encoding_rs の変換（UTF-8 と JIS は自前なので None）。
    fn encoding(self) -> Option<&'static Encoding> {
        Some(match self {
            Charset::Utf8 | Charset::Jis => return None,
            Charset::ShiftJis => SHIFT_JIS,
            Charset::EucJp => EUC_JP,
            Charset::EucKr => EUC_KR,
            Charset::Gbk => GBK,
            Charset::Big5 => BIG5,
            Charset::Koi8R => KOI8_R,
            Charset::Windows1251 => WINDOWS_1251,
            Charset::Ibm866 => IBM866,
            Charset::Windows1252 => WINDOWS_1252,
            Charset::Iso8859_2 => ISO_8859_2,
            Charset::Iso8859_15 => ISO_8859_15,
        })
    }
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
        let inner = match (charset, charset.encoding()) {
            (Charset::Jis, _) => Inner::Jis(JisDecoder::default()),
            (_, Some(e)) => Inner::Legacy(e.new_decoder_without_bom_handling()),
            (_, None) => Inner::Utf8,
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
                    [0x1b, b'(', b'B'] | [0x1b, b'(', b'J'] | [0x1b, b'(', b'H'] => {
                        self.switch(JisSet::Ascii)
                    }
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
                // 8 ビットの半角カナ（Tera Term の KatakanaReceive=8）
                0xa1..=0xdf => {
                    self.lead = None;
                    out.push(char::from_u32(0xff61 + u32::from(b - 0xa1)).unwrap_or('?'));
                }
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
    encode_send_with(charset, JisOptions::default(), bytes)
}

/// JIS で送るときの切り替えの書き方（Tera Term の「設定 > 端末」の漢字イン・漢字アウト・半角カナ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JisOptions {
    /// 漢字イン：`B` なら ESC $ B、`@` なら ESC $ @（KanjiIn）
    pub kanji_in: u8,
    /// 漢字アウト：`B` なら ESC ( B、`J` なら ESC ( J、`H` なら ESC ( H（KanjiOut）
    pub kanji_out: u8,
    /// 半角カナを 7 ビット（SO / SI で切り替え）で送る。false なら 8 ビット（KatakanaSend）
    pub kana_7bit: bool,
}

impl Default for JisOptions {
    /// Tera Term の既定（KanjiIn=B、KanjiOut=B、KatakanaSend=8）
    fn default() -> Self {
        Self {
            kanji_in: b'B',
            kanji_out: b'B',
            kana_7bit: false,
        }
    }
}

/// `encode_send` の JIS の書き方を指定するもの。
pub fn encode_send_with(charset: Charset, jis: JisOptions, bytes: Vec<u8>) -> Vec<u8> {
    let encoding: &'static Encoding = match (charset, charset.encoding()) {
        (Charset::Jis, _) => return encode_jis(&String::from_utf8_lossy(&bytes), jis),
        (_, Some(e)) => e,
        (_, None) => return bytes,
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

/// JIS（ISO-2022-JP）。漢字は漢字イン（ESC $ B か ESC $ @）で入り、漢字アウト（ESC ( B / J / H）で出る。
/// 半角カナは 8 ビットのまま、7 ビットなら SO … SI で囲む。
fn encode_jis(text: &str, jis: JisOptions) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    let mut kanji = false;
    let mut shifted = false;
    let kanji_in = [0x1b, b'$', if jis.kanji_in == b'@' { b'@' } else { b'B' }];
    let kanji_out = [
        0x1b,
        b'(',
        match jis.kanji_out {
            b'J' => b'J',
            b'H' => b'H',
            _ => b'B',
        },
    ];
    let leave = |out: &mut Vec<u8>, kanji: &mut bool, shifted: &mut bool| {
        if *shifted {
            out.push(0x0f); // SI
            *shifted = false;
        }
        if *kanji {
            out.extend_from_slice(&kanji_out);
            *kanji = false;
        }
    };
    for ch in text.chars() {
        let c = ch as u32;
        if ch.is_ascii() {
            leave(&mut out, &mut kanji, &mut shifted);
            out.push(ch as u8);
        } else if (0xff61..=0xff9f).contains(&c) {
            if kanji {
                out.extend_from_slice(&kanji_out);
                kanji = false;
            }
            if jis.kana_7bit {
                if !shifted {
                    out.push(0x0e); // SO
                    shifted = true;
                }
                out.push((c - 0xff61 + 0x21) as u8);
            } else {
                out.push((c - 0xff61 + 0xa1) as u8);
            }
        } else {
            let mut buf = [0u8; 4];
            let (euc, _, had_errors) = EUC_JP.encode(ch.encode_utf8(&mut buf));
            match (&euc[..], had_errors) {
                ([a @ 0xa1..=0xfe, b @ 0xa1..=0xfe], false) => {
                    if shifted {
                        out.push(0x0f);
                        shifted = false;
                    }
                    if !kanji {
                        out.extend_from_slice(&kanji_in);
                        kanji = true;
                    }
                    out.push(a & 0x7f);
                    out.push(b & 0x7f);
                }
                _ => {
                    leave(&mut out, &mut kanji, &mut shifted);
                    out.push(b'?');
                }
            }
        }
    }
    leave(&mut out, &mut kanji, &mut shifted);
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
        // JIS は漢字の連なりごとに囲む。半角カナは既定（Tera Term と同じ）で 8 ビット
        assert_eq!(
            encode_send(Charset::Jis, "日本a".as_bytes().to_vec()),
            b"\x1b$BF|K\\\x1b(Ba"
        );
        assert_eq!(encode_send(Charset::Jis, "ｱ".as_bytes().to_vec()), b"\xb1");
        // 漢字イン ESC $ @、漢字アウト ESC ( J、半角カナは SO … SI
        let jis = JisOptions {
            kanji_in: b'@',
            kanji_out: b'J',
            kana_7bit: true,
        };
        assert_eq!(
            encode_send_with(Charset::Jis, jis, "日ｱｲa".as_bytes().to_vec()),
            b"\x1b$@F|\x1b(J\x0e12\x0fa"
        );
        // 受信は 8 ビットの半角カナと ESC ( H も読む
        assert_eq!(decode_all(Charset::Jis, &[b"\xb1\x1b$BF|\x1b(Hz"]), "ｱ日z");
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

    #[test]
    fn other_languages_round_trip() {
        for (cs, text) in [
            (Charset::EucKr, "안녕하세요"),
            (Charset::Gbk, "你好"),
            (Charset::Big5, "繁體"),
            (Charset::Koi8R, "Привет"),
            (Charset::Windows1251, "Привет"),
            (Charset::Ibm866, "Привет"),
            (Charset::Windows1252, "café €"),
            (Charset::Iso8859_2, "Łódź"),
            (Charset::Iso8859_15, "€uro"),
        ] {
            let sent = encode_send(cs, text.as_bytes().to_vec());
            assert!(!sent.contains(&b'?'), "{cs:?}");
            let mut d = ReceiveDecoder::new(cs);
            assert_eq!(
                String::from_utf8(d.decode(&sent).into_owned()).unwrap(),
                text,
                "{cs:?}"
            );
        }
        assert_eq!(Charset::ALL.len(), 13);
    }
}
