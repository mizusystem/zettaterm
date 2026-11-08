//! SSH の認証とホスト鍵確認で、利用者に尋ねるための契約（TTSSH の各ダイアログに当たる）。
//!
//! このファイルはメインセッションの持ち物（API 契約）。
//!
//! 呼ばれるのは接続処理のスレッド（UI スレッドではない）。実装側は UI に問い合わせを出し、
//! 答えが出るまでこのスレッドを止めて待ってよい。UI スレッドから接続を開いた場合に固まらないよう、
//! 接続は UI スレッド以外で開くこと。

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
