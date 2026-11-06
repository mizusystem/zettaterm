//! ファイル転送のあいだ、受信を端末ではなく転送プロトコルへ回すための契約。
//!
//! このファイルはメインセッションの持ち物（API 契約）。プロトコルの実装は mt-xfer にあり、
//! ffi がそれをこの [`ByteDivert`] に合わせて Session に渡す（core は mt-xfer に依存しない）。

/// 転送中に受信を受け取る相手。メソッドはコアのスレッドから呼ばれる。
pub trait ByteDivert: Send {
    /// 始めに相手へ送るバイト。
    fn start(&mut self) -> Vec<u8>;
    /// 受信バイトを渡し、相手へ送るバイトを受け取る。
    fn receive(&mut self, data: &[u8]) -> Vec<u8>;
    /// 短い間隔で呼ばれる（時間切れの再送や、送信の続き）。
    fn tick(&mut self) -> Vec<u8>;
    /// 中止の合図。
    fn cancel(&mut self) -> Vec<u8>;
    /// 終わったら true。Session は受信を端末に戻す。
    fn finished(&self) -> bool;
}

/// 相手の ZMODEM が始まったことの検出結果（Tera Term の ZMODEM 自動切り替え）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferRequest {
    /// 相手で `sz` が動いた（こちらが受け取る）
    ZmodemReceive,
    /// 相手で `rz` が動いた（こちらから送る）
    ZmodemSend,
}

/// 受信の中から ZMODEM の開始（ZRQINIT / ZRINIT の 16 進ヘッダ）を探す。チャンクの境目もまたぐ。
#[derive(Default)]
pub(crate) struct ZmodemDetector {
    tail: Vec<u8>,
}

/// `** ZDLE B 0`。続く 1 文字で 0（ZRQINIT）か 1（ZRINIT）を見分ける
const PREFIX: &[u8] = b"**\x18B0";

impl ZmodemDetector {
    /// 見つかったら、`bytes` のうち端末に出してよい長さと、ヘッダから後ろのバイト列（前のチャンクから
    /// 続いていればその分も含む。転送に渡す）と、種類を返す。
    pub(crate) fn find(&mut self, bytes: &[u8]) -> Option<(usize, Vec<u8>, TransferRequest)> {
        let mut joined = std::mem::take(&mut self.tail);
        let carried = joined.len();
        joined.extend_from_slice(bytes);
        let found = joined
            .windows(PREFIX.len() + 1)
            .enumerate()
            .find_map(|(i, w)| {
                if &w[..PREFIX.len()] != PREFIX {
                    return None;
                }
                let kind = match w[PREFIX.len()] {
                    b'0' => TransferRequest::ZmodemReceive,
                    b'1' => TransferRequest::ZmodemSend,
                    _ => return None,
                };
                Some((i, kind))
            });
        match found {
            Some((i, kind)) => Some((i.saturating_sub(carried), joined[i..].to_vec(), kind)),
            None => {
                let keep = joined.len().min(PREFIX.len());
                self.tail = joined[joined.len() - keep..].to_vec();
                None
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.tail.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_zrqinit_and_zrinit() {
        let mut d = ZmodemDetector::default();
        assert_eq!(
            d.find(b"$ sz a.txt\r\nrz\r**\x18B00000000000000\r\n"),
            Some((
                15,
                b"**\x18B00000000000000\r\n".to_vec(),
                TransferRequest::ZmodemReceive
            ))
        );
        let mut d = ZmodemDetector::default();
        assert_eq!(
            d.find(b"**\x18B0100000023be50\r\n"),
            Some((
                0,
                b"**\x18B0100000023be50\r\n".to_vec(),
                TransferRequest::ZmodemSend
            ))
        );
        let mut d = ZmodemDetector::default();
        assert_eq!(d.find(b"ordinary output ** B00"), None);
    }

    #[test]
    fn finds_across_chunks() {
        let mut d = ZmodemDetector::default();
        assert_eq!(d.find(b"rz\r**\x18"), None);
        // 前のチャンクに入っていたヘッダの頭も、転送に渡す側に入る
        assert_eq!(
            d.find(b"B00000"),
            Some((0, b"**\x18B00000".to_vec(), TransferRequest::ZmodemReceive))
        );
    }
}
