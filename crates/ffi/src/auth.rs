//! SSH の認証・ホスト鍵確認の問い合わせ（コアの AuthPrompter を UI に渡す）。
//! UI 側の実装は接続処理のスレッドから呼ばれるので、メインスレッドに問い合わせを出して答えを待つ。

use std::sync::Arc;

use mt_core as core;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HostKeyStatus {
    Unknown,
    Changed,
    DifferentType,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub status: HostKeyStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HostKeyDecision {
    Reject,
    AcceptOnce,
    AcceptAndSave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AuthMethod {
    Password,
    PublicKey,
    KeyboardInteractive,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AuthRequest {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub methods: Vec<AuthMethod>,
    pub default_key: Option<String>,
    pub failed: bool,
    pub further: bool,
    pub banner: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AuthResponse {
    Cancel,
    Password {
        user: String,
        password: String,
    },
    PublicKey {
        user: String,
        key_path: String,
        passphrase: Option<String>,
    },
    KeyboardInteractive {
        user: String,
    },
    Agent {
        user: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct KeyboardPrompt {
    pub text: String,
    pub echo: bool,
}

/// UI が実装する問い合わせ窓口（TTSSH の「セキュリティ警告」「SSH認証」「SSH 認証チャレンジ」）。
#[uniffi::export(with_foreign)]
pub trait SshPrompter: Send + Sync {
    fn host_key(&self, info: HostKeyInfo) -> HostKeyDecision;
    fn credentials(&self, request: AuthRequest) -> AuthResponse;
    /// None なら中止
    fn keyboard_interactive(
        &self,
        name: String,
        instructions: String,
        prompts: Vec<KeyboardPrompt>,
    ) -> Option<Vec<String>>;
    /// None なら、その鍵は飛ばす
    fn passphrase(&self, key_path: String, failed: bool) -> Option<String>;
}

pub(crate) struct PrompterAdapter(pub Arc<dyn SshPrompter>);

impl core::AuthPrompter for PrompterAdapter {
    fn host_key(&self, info: core::HostKeyInfo) -> core::HostKeyDecision {
        let info = HostKeyInfo {
            host: info.host,
            port: info.port,
            algorithm: info.algorithm,
            fingerprint: info.fingerprint,
            status: match info.status {
                core::HostKeyStatus::Unknown => HostKeyStatus::Unknown,
                core::HostKeyStatus::Changed => HostKeyStatus::Changed,
                core::HostKeyStatus::DifferentType => HostKeyStatus::DifferentType,
            },
        };
        match self.0.host_key(info) {
            HostKeyDecision::Reject => core::HostKeyDecision::Reject,
            HostKeyDecision::AcceptOnce => core::HostKeyDecision::AcceptOnce,
            HostKeyDecision::AcceptAndSave => core::HostKeyDecision::AcceptAndSave,
        }
    }

    fn credentials(&self, r: core::AuthRequest) -> core::AuthResponse {
        let request = AuthRequest {
            host: r.host,
            port: r.port,
            user: r.user,
            methods: r
                .methods
                .into_iter()
                .map(|m| match m {
                    core::AuthMethod::Password => AuthMethod::Password,
                    core::AuthMethod::PublicKey => AuthMethod::PublicKey,
                    core::AuthMethod::KeyboardInteractive => AuthMethod::KeyboardInteractive,
                    core::AuthMethod::Agent => AuthMethod::Agent,
                })
                .collect(),
            default_key: r.default_key,
            failed: r.failed,
            further: r.further,
            banner: r.banner,
        };
        match self.0.credentials(request) {
            AuthResponse::Cancel => core::AuthResponse::Cancel,
            AuthResponse::Password { user, password } => {
                core::AuthResponse::Password { user, password }
            }
            AuthResponse::PublicKey {
                user,
                key_path,
                passphrase,
            } => core::AuthResponse::PublicKey {
                user,
                key_path,
                passphrase,
            },
            AuthResponse::KeyboardInteractive { user } => {
                core::AuthResponse::KeyboardInteractive { user }
            }
            AuthResponse::Agent { user } => core::AuthResponse::Agent { user },
        }
    }

    fn keyboard_interactive(
        &self,
        name: String,
        instructions: String,
        prompts: Vec<core::KeyboardPrompt>,
    ) -> Option<Vec<String>> {
        let prompts = prompts
            .into_iter()
            .map(|p| KeyboardPrompt {
                text: p.text,
                echo: p.echo,
            })
            .collect();
        self.0.keyboard_interactive(name, instructions, prompts)
    }

    fn passphrase(&self, key_path: String, failed: bool) -> Option<String> {
        self.0.passphrase(key_path, failed)
    }
}
