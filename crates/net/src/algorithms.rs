//! 内蔵 SSH が交渉に出すアルゴリズムの優先順位（TTSSH の「SSH 設定」の暗号・鍵交換・MAC の順番と圧縮）。
//!
//! 書いた名前を前に、書かなかったものは既定の順で後ろに並べる（消しはしない。鍵交換の拡張の印なども残る）。
//! 知らない名前は無視して、呼び出し元に返す。

use std::borrow::Cow;
use std::sync::Mutex;

use russh::{Preferred, cipher, compression, kex, mac};

/// 優先順位の設定。空の並びは既定のまま。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlgorithmPrefs {
    pub kex: Vec<String>,
    pub cipher: Vec<String>,
    pub mac: Vec<String>,
    /// 圧縮を使う（zlib@openssh.com / zlib）
    pub compression: bool,
}

static CURRENT: Mutex<Option<AlgorithmPrefs>> = Mutex::new(None);

/// 既定の順番（設定画面に出す）。
pub fn defaults() -> AlgorithmPrefs {
    let d = Preferred::DEFAULT;
    AlgorithmPrefs {
        kex: d
            .kex
            .iter()
            .map(|n| n.as_ref().to_owned())
            .filter(|n| !is_marker(n))
            .collect(),
        cipher: d.cipher.iter().map(|n| n.as_ref().to_owned()).collect(),
        mac: d.mac.iter().map(|n| n.as_ref().to_owned()).collect(),
        compression: false,
    }
}

/// 鍵交換の並びにある、アルゴリズムではない印（ext-info-c、kex-strict-c-v00@openssh.com）。
fn is_marker(name: &str) -> bool {
    name.starts_with("ext-info") || name.starts_with("kex-strict")
}

/// これからの接続に使う（None で既定）。知らない名前を返す。
pub fn set(prefs: Option<AlgorithmPrefs>) -> Vec<String> {
    let mut unknown = Vec::new();
    if let Some(p) = &prefs {
        for n in &p.kex {
            if kex::Name::try_from(n.as_str()).is_err() {
                unknown.push(n.clone());
            }
        }
        for n in &p.cipher {
            if cipher::Name::try_from(n.as_str()).is_err() {
                unknown.push(n.clone());
            }
        }
        for n in &p.mac {
            if mac::Name::try_from(n.as_str()).is_err() {
                unknown.push(n.clone());
            }
        }
    }
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = prefs;
    unknown
}

/// 書いた名前（読めたもの）を前に、残りを既定の順で後ろに。
fn reorder<N: Copy + PartialEq>(
    wanted: &[String],
    parse: impl Fn(&str) -> Option<N>,
    default: &[N],
) -> Vec<N> {
    let mut out: Vec<N> = Vec::new();
    for w in wanted {
        if let Some(n) = parse(w.trim())
            && default.contains(&n)
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    for n in default {
        if !out.contains(n) {
            out.push(*n);
        }
    }
    out
}

/// 接続に渡す優先順位。
pub(crate) fn preferred() -> Preferred {
    let Some(p) = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return Preferred::DEFAULT;
    };
    let d = Preferred::DEFAULT;
    Preferred {
        kex: Cow::Owned(reorder(&p.kex, |s| kex::Name::try_from(s).ok(), &d.kex)),
        cipher: Cow::Owned(reorder(
            &p.cipher,
            |s| cipher::Name::try_from(s).ok(),
            &d.cipher,
        )),
        mac: Cow::Owned(reorder(&p.mac, |s| mac::Name::try_from(s).ok(), &d.mac)),
        compression: if p.compression {
            Preferred::COMPRESSED.compression
        } else {
            Cow::Borrowed(&[compression::NONE])
        },
        ..d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_names_go_first_and_nothing_is_dropped() {
        let d = defaults();
        assert!(d.cipher.len() > 2 && !d.kex.iter().any(|n| is_marker(n)));
        let last = d.cipher.last().unwrap().clone();
        let unknown = set(Some(AlgorithmPrefs {
            cipher: vec![last.clone(), "rot13".into()],
            compression: true,
            ..Default::default()
        }));
        assert_eq!(unknown, ["rot13"]);
        let p = preferred();
        assert_eq!(p.cipher[0].as_ref(), last);
        assert_eq!(p.cipher.len(), Preferred::DEFAULT.cipher.len());
        // 鍵交換は既定のまま（拡張の印も残る）
        assert_eq!(p.kex.len(), Preferred::DEFAULT.kex.len());
        assert!(p.compression.len() > 1);
        set(None);
        assert_eq!(preferred().cipher, Preferred::DEFAULT.cipher);
    }
}
