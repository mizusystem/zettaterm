//! SSH の認証とホスト鍵確認で、利用者に尋ねるための契約（TTSSH の各ダイアログに当たる）。
//!
//! このファイルはメインセッションの持ち物（API 契約）。
//!
//! 呼ばれるのは接続処理のスレッド（UI スレッドではない）。実装側は UI に問い合わせを出し、
//! 答えが出るまでこのスレッドを止めて待ってよい。UI スレッドから接続を開いた場合に固まらないよう、
//! 接続は UI スレッド以外で開くこと。

use std::fmt;

/// known_hosts とサーバーのホスト鍵の照合結果（TTSSH の「セキュリティ警告」の 3 種類）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostKeyStatus {
    /// known_hosts にこのホストが無い
    Unknown,
    /// 同じ種類の鍵が記録されているが、一致しない（なりすましの可能性）
    Changed,
    /// 別の種類の鍵だけが記録されている
    DifferentType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKeyInfo {
    /// known_hosts に書く名前（ssh_config の HostName 解決後）
    pub host: String,
    pub port: u16,
    /// 鍵の種類（ssh-ed25519、ecdsa-sha2-nistp256 など）
    pub algorithm: String,
    /// `SHA256:...` 形式の指紋
    pub fingerprint: String,
    pub status: HostKeyStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostKeyDecision {
    /// 接続をやめる
    Reject,
    /// 今回だけ受け入れる
    AcceptOnce,
    /// 受け入れて known_hosts に書く（Changed なら置き換える）
    AcceptAndSave,
}

/// 認証方式（TTSSH の「認証方式」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthMethod {
    /// プレインパスワード
    Password,
    /// RSA / ECDSA / ED25519 の鍵ファイル
    PublicKey,
    /// キーボードインタラクティブ（ワンタイムパスワードなど）
    KeyboardInteractive,
    /// ssh-agent（Windows では Pageant に当たる）
    Agent,
}

/// 認証ダイアログに出す内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequest {
    pub host: String,
    pub port: u16,
    /// 既定のユーザー名（ssh_config の User、指定、ログオンユーザー名の順）
    pub user: String,
    /// サーバーが受け付けると言っている方式
    pub methods: Vec<AuthMethod>,
    /// ssh_config の IdentityFile のうち最初のもの（鍵ファイル欄の初期値）
    pub default_key: Option<String>,
    /// 直前の試行が失敗した（TTSSH の「認証に失敗しました. 再試行してください.」）
    pub failed: bool,
    /// 認証は通ったが、さらに別の方式が要る（TTSSH の「追加の認証が必要です.」）
    pub further: bool,
    /// サーバーが送ってきたバナー
    pub banner: Option<String>,
}

/// 認証ダイアログの答え。
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// キーボードインタラクティブ認証の 1 項目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardPrompt {
    pub text: String,
    /// 入力を画面に出してよいか（false ならパスワード入力として伏せる）
    pub echo: bool,
}

/// パスワード認証の途中で、サーバーがパスワードの変更を求めてきた（TTSSH の「パスワード変更」。
/// SSH_MSG_USERAUTH_PASSWD_CHANGEREQ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordChangeRequest {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// サーバーの文言（空のこともある）
    pub prompt: String,
    /// 直前に送った新しいパスワードが受け付けられなかった（もう一度尋ねている）
    pub failed: bool,
}

/// パスワード変更ダイアログの答え（新パスワードの確認欄との一致は UI が確かめる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordChange {
    pub old_password: String,
    pub new_password: String,
}

/// 利用者に尋ねる窓口。UI 側が実装する。
pub trait AuthPrompter: Send + Sync {
    /// known_hosts と合わないホスト鍵を受け入れるか。
    fn host_key(&self, info: HostKeyInfo) -> HostKeyDecision;

    /// 自動の認証（エージェント、ssh_config の鍵）で通らなかったときに、認証ダイアログを出す。
    fn credentials(&self, request: AuthRequest) -> AuthResponse;

    /// キーボードインタラクティブ認証の質問に答える。None なら中止。
    fn keyboard_interactive(
        &self,
        name: String,
        instructions: String,
        prompts: Vec<KeyboardPrompt>,
    ) -> Option<Vec<String>>;

    /// パスフレーズ付きの鍵（ssh_config の IdentityFile など）を自動で試すときのパスフレーズ。
    /// None なら、その鍵は飛ばす。
    fn passphrase(&self, key_path: String, failed: bool) -> Option<String>;

    /// サーバーがパスワードの変更を求めてきたときに、旧・新のパスワードを尋ねる。None なら中止。
    fn password_change(&self, request: PasswordChangeRequest) -> Option<PasswordChange> {
        let _ = request;
        None
    }
}

/// マクロの `connect` や起動引数の `/auth=` `/keyfile=` `/passwd=`（TTSSH の自動ログイン）。
/// 接続ごとに作り、最初の認証で 1 回だけ使う。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LoginPreset {
    /// `password` / `publickey` / `challenge` / `pageant`（無ければ password）
    pub auth: Option<String>,
    pub key_file: Option<String>,
    /// パスワード、または鍵のパスフレーズ
    pub password: Option<String>,
}

impl fmt::Debug for LoginPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // パスワードはログに出さない
        f.debug_struct("LoginPreset")
            .field("auth", &self.auth)
            .field("key_file", &self.key_file)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

impl LoginPreset {
    /// ダイアログを出さずに答えられるなら、その答え（使ったぶんは消す）。答えられないときは
    /// `None` で、鍵ファイルを認証ダイアログの初期値に入れておく。
    ///
    /// TTSSH と同じく、パスワードと鍵はパスワード（パスフレーズ）まで揃ったときだけ自動で入る。
    /// ssh-agent（Pageant）は `/auth=pageant` だけで入る。
    pub fn answer(&mut self, request: &mut AuthRequest) -> Option<AuthResponse> {
        if let Some(k) = &self.key_file {
            request.default_key = Some(k.clone());
        }
        if request.failed || request.further {
            // 失敗したら、もう同じものは使わない
            self.password = None;
            self.auth = None;
            return None;
        }
        let user = request.user.clone();
        let offered = |m: AuthMethod| request.methods.contains(&m);
        match self.auth.as_deref().unwrap_or("password") {
            "password" if offered(AuthMethod::Password) => {
                let password = self.password.take()?;
                Some(AuthResponse::Password { user, password })
            }
            "publickey" if offered(AuthMethod::PublicKey) => {
                let key_path = self.key_file.clone()?;
                let pass = self.password.take()?;
                self.auth = None;
                Some(AuthResponse::PublicKey {
                    user,
                    key_path,
                    passphrase: (!pass.is_empty()).then_some(pass),
                })
            }
            "pageant" if offered(AuthMethod::Agent) => {
                self.auth = None;
                Some(AuthResponse::Agent { user })
            }
            _ => None,
        }
    }
}

/// 何も尋ねられない場面（テストや CLI の非対話実行）用。未知のホスト鍵は拒否し、認証は中止する。
pub struct NoPrompt;

impl AuthPrompter for NoPrompt {
    fn host_key(&self, _: HostKeyInfo) -> HostKeyDecision {
        HostKeyDecision::Reject
    }
    fn credentials(&self, _: AuthRequest) -> AuthResponse {
        AuthResponse::Cancel
    }
    fn keyboard_interactive(
        &self,
        _: String,
        _: String,
        _: Vec<KeyboardPrompt>,
    ) -> Option<Vec<String>> {
        None
    }
    fn passphrase(&self, _: String, _: bool) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod login_preset_tests {
    use super::*;

    fn request(methods: Vec<AuthMethod>) -> AuthRequest {
        AuthRequest {
            host: "h".into(),
            port: 22,
            user: "u".into(),
            methods,
            default_key: None,
            failed: false,
            further: false,
            banner: None,
        }
    }

    #[test]
    fn password_is_used_once() {
        let mut p = LoginPreset {
            password: Some("pw".into()),
            ..Default::default()
        };
        let mut r = request(vec![AuthMethod::Password]);
        assert_eq!(
            p.answer(&mut r),
            Some(AuthResponse::Password {
                user: "u".into(),
                password: "pw".into()
            })
        );
        assert_eq!(p.answer(&mut r), None);
    }

    #[test]
    fn key_needs_the_passphrase_like_ttssh_otherwise_the_dialog_gets_the_file() {
        let mut p = LoginPreset {
            auth: Some("publickey".into()),
            key_file: Some("/k".into()),
            password: None,
        };
        let mut r = request(vec![AuthMethod::PublicKey, AuthMethod::Agent]);
        assert_eq!(p.answer(&mut r), None);
        assert_eq!(r.default_key.as_deref(), Some("/k"));

        let mut p = LoginPreset {
            auth: Some("publickey".into()),
            key_file: Some("/k".into()),
            password: Some(String::new()),
        };
        assert_eq!(
            p.answer(&mut request(vec![AuthMethod::PublicKey])),
            Some(AuthResponse::PublicKey {
                user: "u".into(),
                key_path: "/k".into(),
                passphrase: None
            })
        );
    }

    #[test]
    fn agent_and_failures() {
        let mut p = LoginPreset {
            auth: Some("pageant".into()),
            ..Default::default()
        };
        let mut r = request(vec![AuthMethod::PublicKey, AuthMethod::Agent]);
        assert_eq!(
            p.answer(&mut r),
            Some(AuthResponse::Agent { user: "u".into() })
        );
        // 一度使ったら（失敗したら）ダイアログに任せる
        assert_eq!(p.answer(&mut r), None);
        let mut p = LoginPreset {
            password: Some("pw".into()),
            ..Default::default()
        };
        let mut r = request(vec![AuthMethod::Password]);
        r.failed = true;
        assert_eq!(p.answer(&mut r), None);
        assert_eq!(p.password, None);
    }
}
