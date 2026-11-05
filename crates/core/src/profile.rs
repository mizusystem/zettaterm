//! 接続プロファイルとホスト履歴（Tera Term の「ホストリスト」）。TOML で保存する。
//!
//! ```toml
//! [[recent]]
//! type = "ssh"
//! host = "web"
//!
//! [[recent]]
//! type = "serial"
//! port = "/dev/cu.usbmodem1101"
//! settings = { baud = 115200 }
//! ```

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::transport::{SendDelay, SerialSettings};

/// 履歴に残す件数の上限
pub const MAX_RECENT: usize = 50;

/// 接続先。UI の「新しい接続」で選ぶものと 1 対 1 に対応する。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionProfile {
    LocalShell,
    /// SSH。`host` は ssh_config の別名や `user@host` でもよい。
    /// `system_ssh` ならシステムの ssh を使う（既定は内蔵の SSH）
    Ssh {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        system_ssh: bool,
    },
    Telnet {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    /// 生の TCP（Tera Term の「その他」）
    Tcp {
        host: String,
        port: u16,
    },
    Serial {
        port: String,
        #[serde(default)]
        settings: SerialSettings,
        #[serde(default)]
        delay: SendDelay,
    },
}

impl ConnectionProfile {
    /// 履歴で同じ接続先とみなすためのキー（シリアルは通信設定が違っても同じポートなら同じ）。
    fn same_target(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Serial { port: a, .. }, Self::Serial { port: b, .. }) => a == b,
            _ => self == other,
        }
    }

    /// 一覧やタイトルに出す短い名前。
    pub fn label(&self) -> String {
        match self {
            Self::LocalShell => "local shell".to_owned(),
            Self::Ssh {
                host, port, user, ..
            } => {
                let mut s = String::new();
                if let Some(u) = user {
                    s.push_str(u);
                    s.push('@');
                }
                s.push_str(host);
                if let Some(p) = port {
                    s.push_str(&format!(":{p}"));
                }
                s
            }
            Self::Telnet { host, port } => match port {
                Some(p) => format!("{host}:{p}"),
                None => host.clone(),
            },
            Self::Tcp { host, port } => format!("{host}:{port}"),
            Self::Serial { port, .. } => port.clone(),
        }
    }
}

/// 最近つないだ接続先。先頭が最新。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostHistory {
    #[serde(default)]
    pub recent: Vec<ConnectionProfile>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl HostHistory {
    /// `path` から読む。ファイルが無ければ空。壊れていればエラー。
    pub fn load(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let mut h = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<HostHistory>(&text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => HostHistory::default(),
            Err(e) => return Err(e),
        };
        h.path = Some(path);
        Ok(h)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 先頭に足す。同じ接続先があれば先頭へ動かす。
    pub fn push(&mut self, profile: ConnectionProfile) {
        self.recent.retain(|p| !p.same_target(&profile));
        self.recent.insert(0, profile);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.recent.len() {
            self.recent.remove(index);
        }
    }

    /// 読んだファイルに書き戻す。親ディレクトリが無ければ作る。
    pub fn save(&self) -> io::Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| io::Error::other("no path"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| io::Error::other(e.to_string()))?;
        // 書きかけのファイルを残さないよう、一時ファイルに書いてから置き換える
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Parity;

    fn ssh(host: &str) -> ConnectionProfile {
        ConnectionProfile::Ssh {
            host: host.into(),
            port: None,
            user: None,
            system_ssh: false,
        }
    }

    #[test]
    fn push_dedupes_and_keeps_newest_first() {
        let mut h = HostHistory::default();
        h.push(ssh("a"));
        h.push(ssh("b"));
        h.push(ssh("a"));
        assert_eq!(h.recent, [ssh("a"), ssh("b")]);

        // シリアルは同じポートなら設定が違っても 1 件
        let serial = |baud| ConnectionProfile::Serial {
            port: "/dev/cu.x".into(),
            settings: SerialSettings {
                baud,
                ..Default::default()
            },
            delay: SendDelay::default(),
        };
        h.push(serial(9600));
        h.push(serial(115_200));
        assert_eq!(h.recent.len(), 3);
        assert_eq!(h.recent[0], serial(115_200));

        for i in 0..100 {
            h.push(ssh(&format!("h{i}")));
        }
        assert_eq!(h.recent.len(), MAX_RECENT);
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("hosts.toml");
        let mut h = HostHistory::load(&path).unwrap();
        assert!(h.recent.is_empty());
        h.push(ConnectionProfile::LocalShell);
        h.push(ConnectionProfile::Telnet {
            host: "bbs".into(),
            port: Some(2323),
        });
        h.push(ConnectionProfile::Tcp {
            host: "10.0.0.2".into(),
            port: 7,
        });
        h.push(ConnectionProfile::Ssh {
            host: "web".into(),
            port: Some(2222),
            user: Some("kt".into()),
            system_ssh: true,
        });
        h.push(ConnectionProfile::Serial {
            port: "/dev/cu.usbmodem1101".into(),
            settings: SerialSettings {
                baud: 115_200,
                parity: Parity::Even,
                ..Default::default()
            },
            delay: SendDelay {
                per_char_ms: 1,
                per_line_ms: 0,
            },
        });
        h.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("type = \"ssh\""), "{text}");
        assert_eq!(HostHistory::load(&path).unwrap().recent, h.recent);
    }

    #[test]
    fn reads_hand_written_files_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.toml");
        std::fs::write(
            &path,
            "[[recent]]\ntype = \"ssh\"\nhost = \"alias\"\n\n[[recent]]\ntype = \"serial\"\nport = \"COM3\"\nsettings = { baud = 115200 }\n",
        )
        .unwrap();
        let h = HostHistory::load(&path).unwrap();
        assert_eq!(h.recent[0], ssh("alias"));
        match &h.recent[1] {
            ConnectionProfile::Serial { settings, .. } => {
                assert_eq!(settings.baud, 115_200);
                assert_eq!(settings.data_bits, 8);
            }
            other => panic!("{other:?}"),
        }
        std::fs::write(&path, "recent = 3").unwrap();
        assert_eq!(
            HostHistory::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn labels() {
        assert_eq!(
            ConnectionProfile::Ssh {
                host: "h".into(),
                port: Some(22),
                user: Some("u".into()),
                system_ssh: false,
            }
            .label(),
            "u@h:22"
        );
        assert_eq!(
            ConnectionProfile::Tcp {
                host: "h".into(),
                port: 7
            }
            .label(),
            "h:7"
        );
    }
}
