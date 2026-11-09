//! ホスト履歴（Tera Term の「ホストリスト」）と ssh_config のホスト一覧。

use std::sync::Arc;
use std::sync::Mutex;

use zetta_core as core;

use crate::{IpVersion, MtError, Profile, SerialSettings};

impl Profile {
    /// 履歴に残す形。ローカルシェルの引数と IP バージョンは残さない。
    pub(crate) fn to_core(&self) -> core::ConnectionProfile {
        match self {
            Profile::LocalShell { .. } => core::ConnectionProfile::LocalShell,
            Profile::Replay { path, .. } => core::ConnectionProfile::Replay { path: path.clone() },
            Profile::Unix { path } => core::ConnectionProfile::Unix { path: path.clone() },
            Profile::Serial { port, settings } => core::ConnectionProfile::Serial {
                port: port.clone(),
                settings: settings.port(),
                delay: settings.delay(),
            },
            Profile::Ssh {
                host,
                port,
                user,
                system_ssh,
            } => core::ConnectionProfile::Ssh {
                host: host.clone(),
                port: *port,
                user: user.clone(),
                system_ssh: *system_ssh,
            },
            Profile::Telnet { host, port, .. } => core::ConnectionProfile::Telnet {
                host: host.clone(),
                port: *port,
            },
            Profile::Tcp { host, port, .. } => core::ConnectionProfile::Tcp {
                host: host.clone(),
                port: *port,
            },
        }
    }

    pub(crate) fn from_core(p: core::ConnectionProfile) -> Self {
        match p {
            core::ConnectionProfile::LocalShell => Profile::LocalShell {
                program: None,
                args: Vec::new(),
                cwd: None,
            },
            core::ConnectionProfile::Serial {
                port,
                settings,
                delay,
            } => Profile::Serial {
                port,
                settings: SerialSettings::from_core(settings, delay),
            },
            core::ConnectionProfile::Ssh {
                host,
                port,
                user,
                system_ssh,
            } => Profile::Ssh {
                host,
                port,
                user,
                system_ssh,
            },
            core::ConnectionProfile::Telnet { host, port } => Profile::Telnet {
                host,
                port,
                ip: IpVersion::Auto,
            },
            core::ConnectionProfile::Replay { path } => Profile::Replay { path, speed: 1.0 },
            core::ConnectionProfile::Unix { path } => Profile::Unix { path },
            core::ConnectionProfile::Tcp { host, port } => Profile::Tcp {
                host,
                port,
                ip: IpVersion::Auto,
            },
        }
    }
}

/// 最近つないだ接続先。TOML ファイルに保存する。
#[derive(uniffi::Object)]
pub struct HostHistory {
    inner: Mutex<core::HostHistory>,
}

#[uniffi::export]
impl HostHistory {
    /// `path` の履歴を開く（無ければ空）。中身が壊れていればエラー。
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, MtError> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::HostHistory::load(path)?),
        }))
    }

    /// 先頭が最新。
    pub fn entries(&self) -> Vec<Profile> {
        self.lock()
            .recent
            .iter()
            .cloned()
            .map(Profile::from_core)
            .collect()
    }

    /// 先頭に足して保存する。同じ接続先は先頭へ動かす。
    pub fn add(&self, profile: Profile) -> Result<(), MtError> {
        // ログの再生は接続先ではないので残さない
        if matches!(profile, Profile::Replay { .. }) {
            return Ok(());
        }
        let mut h = self.lock();
        h.push(profile.to_core());
        Ok(h.save()?)
    }

    pub fn remove(&self, index: u32) -> Result<(), MtError> {
        let mut h = self.lock();
        h.remove(index as usize);
        Ok(h.save()?)
    }

    /// 並べ替える（上へ・下へ）。
    pub fn move_item(&self, from: u32, to: u32) -> Result<(), MtError> {
        let mut h = self.lock();
        h.move_item(from as usize, to as usize);
        Ok(h.save()?)
    }
}

impl HostHistory {
    fn lock(&self) -> std::sync::MutexGuard<'_, core::HostHistory> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 利用者の `~/.ssh/config` にある Host 名（ワイルドカードを除く）。
#[uniffi::export]
pub fn ssh_config_hosts() -> Vec<String> {
    zetta_net::ssh_config_hosts()
}
