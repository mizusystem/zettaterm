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

/// サーバーがパスワードの変更を求めてきた（TTSSH の「パスワード変更」）。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PasswordChangeRequest {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// サーバーの文言（空のこともある）
    pub prompt: String,
    /// 直前の新しいパスワードが受け付けられなかった
    pub failed: bool,
}

/// パスワード変更ダイアログの答え（確認欄との一致は UI が確かめる）。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PasswordChange {
    pub old_password: String,
    pub new_password: String,
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
    /// None なら中止
    fn password_change(&self, request: PasswordChangeRequest) -> Option<PasswordChange>;
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
        self.0.credentials(r.into()).into()
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

    fn password_change(&self, r: core::PasswordChangeRequest) -> Option<core::PasswordChange> {
        let request = PasswordChangeRequest {
            host: r.host,
            port: r.port,
            user: r.user,
            prompt: r.prompt,
            failed: r.failed,
        };
        self.0
            .password_change(request)
            .map(|c| core::PasswordChange {
                old_password: c.old_password,
                new_password: c.new_password,
            })
    }
}

impl From<core::AuthRequest> for AuthRequest {
    fn from(r: core::AuthRequest) -> Self {
        Self {
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
        }
    }
}

impl From<AuthRequest> for core::AuthRequest {
    fn from(r: AuthRequest) -> Self {
        Self {
            host: r.host,
            port: r.port,
            user: r.user,
            methods: r
                .methods
                .into_iter()
                .map(|m| match m {
                    AuthMethod::Password => core::AuthMethod::Password,
                    AuthMethod::PublicKey => core::AuthMethod::PublicKey,
                    AuthMethod::KeyboardInteractive => core::AuthMethod::KeyboardInteractive,
                    AuthMethod::Agent => core::AuthMethod::Agent,
                })
                .collect(),
            default_key: r.default_key,
            failed: r.failed,
            further: r.further,
            banner: r.banner,
        }
    }
}

impl From<AuthResponse> for core::AuthResponse {
    fn from(r: AuthResponse) -> Self {
        match r {
            AuthResponse::Cancel => Self::Cancel,
            AuthResponse::Password { user, password } => Self::Password { user, password },
            AuthResponse::PublicKey {
                user,
                key_path,
                passphrase,
            } => Self::PublicKey {
                user,
                key_path,
                passphrase,
            },
            AuthResponse::KeyboardInteractive { user } => Self::KeyboardInteractive { user },
            AuthResponse::Agent { user } => Self::Agent { user },
        }
    }
}

impl From<core::AuthResponse> for AuthResponse {
    fn from(r: core::AuthResponse) -> Self {
        match r {
            core::AuthResponse::Cancel => Self::Cancel,
            core::AuthResponse::Password { user, password } => Self::Password { user, password },
            core::AuthResponse::PublicKey {
                user,
                key_path,
                passphrase,
            } => Self::PublicKey {
                user,
                key_path,
                passphrase,
            },
            core::AuthResponse::KeyboardInteractive { user } => Self::KeyboardInteractive { user },
            core::AuthResponse::Agent { user } => Self::Agent { user },
        }
    }
}

/// マクロ・起動引数の `/auth=` `/keyfile=` `/passwd=`（TTSSH の自動ログイン。`mt_core::LoginPreset`）。
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct LoginPreset {
    pub auth: Option<String>,
    pub key_file: Option<String>,
    pub password: Option<String>,
}

/// [`answer_login_preset`] の結果。`response` があればダイアログを出さずにそれで答える。
/// 無ければ `request`（鍵ファイルの初期値を入れたもの）でダイアログを出す。`preset` は次に使う残り。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LoginAnswer {
    pub response: Option<AuthResponse>,
    pub request: AuthRequest,
    pub preset: LoginPreset,
}

/// 認証の問い合わせに、マクロで決めたログインの仕方で答えられるかを見る。
#[uniffi::export]
pub fn answer_login_preset(preset: LoginPreset, request: AuthRequest) -> LoginAnswer {
    let mut p = core::LoginPreset {
        auth: preset.auth,
        key_file: preset.key_file,
        password: preset.password,
    };
    let mut r: core::AuthRequest = request.into();
    let response = p.answer(&mut r).map(Into::into);
    LoginAnswer {
        response,
        request: r.into(),
        preset: LoginPreset {
            auth: p.auth,
            key_file: p.key_file,
            password: p.password,
        },
    }
}
