//! コアのスレッドから GTK のメインスレッドへの橋渡し。
//!
//! - SessionListener：damage はフラグだけ立て（描画はフレームごとに見に行く）、それ以外はチャネルで送る
//! - AuthPrompter：問い合わせをチャネルでメインスレッドへ送り、答えが返るまで接続のスレッドを止める

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use mt_core::{
    AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo, KeyboardPrompt,
    SessionListener, SessionState, TransferRequest,
};

pub enum Event {
    State(SessionState),
    Title(String),
    Bell,
    Transfer(TransferRequest),
    /// 相手がクリップボードに書いた（OSC 52）
    Clipboard(String),
}

pub struct Listener {
    pub dirty: Arc<AtomicBool>,
    pub tx: async_channel::Sender<Event>,
}

impl SessionListener for Listener {
    fn on_damage(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }
    fn on_state(&self, state: SessionState) {
        let _ = self.tx.send_blocking(Event::State(state));
    }
    fn on_bell(&self) {
        let _ = self.tx.send_blocking(Event::Bell);
    }
    fn on_title(&self, title: String) {
        let _ = self.tx.send_blocking(Event::Title(title));
    }
    fn on_transfer_request(&self, request: TransferRequest) {
        let _ = self.tx.send_blocking(Event::Transfer(request));
    }
    fn on_clipboard(&self, text: String) {
        let _ = self.tx.send_blocking(Event::Clipboard(text));
    }
}

/// SSH の問い合わせ 1 件と、答えの返し先。
pub enum Prompt {
    HostKey(HostKeyInfo, mpsc::Sender<HostKeyDecision>),
    Credentials(AuthRequest, mpsc::Sender<AuthResponse>),
    Keyboard {
        name: String,
        instructions: String,
        prompts: Vec<KeyboardPrompt>,
        reply: mpsc::Sender<Option<Vec<String>>>,
    },
    Passphrase {
        key_path: String,
        failed: bool,
        reply: mpsc::Sender<Option<String>>,
    },
}

/// 接続 1 回分の問い合わせ役。接続ごとに作る。
pub struct Prompter {
    pub tx: async_channel::Sender<Prompt>,
    /// マクロの `/passwd=`。この接続の最初のパスワード認証で 1 回だけ使い、接続が終わったら消す
    pub password: std::sync::Mutex<Option<String>>,
}

impl Prompter {
    pub fn new(tx: async_channel::Sender<Prompt>, password: Option<String>) -> Self {
        Self {
            tx,
            password: std::sync::Mutex::new(password),
        }
    }

    /// 使わなかったパスワードを消す（接続の成功・失敗のあと）。
    pub fn forget_password(&self) {
        self.password
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}

impl Prompter {
    fn ask<T>(&self, make: impl FnOnce(mpsc::Sender<T>) -> Prompt, fallback: T) -> T {
        let (reply, rx) = mpsc::channel();
        if self.tx.send_blocking(make(reply)).is_err() {
            return fallback;
        }
        rx.recv().unwrap_or(fallback)
    }
}

impl AuthPrompter for Prompter {
    fn host_key(&self, info: HostKeyInfo) -> HostKeyDecision {
        self.ask(|r| Prompt::HostKey(info, r), HostKeyDecision::Reject)
    }

    fn credentials(&self, request: AuthRequest) -> AuthResponse {
        if !request.failed
            && request.methods.contains(&mt_core::AuthMethod::Password)
            && let Some(password) = self
                .password
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
        {
            return AuthResponse::Password {
                user: request.user,
                password,
            };
        }
        self.ask(|r| Prompt::Credentials(request, r), AuthResponse::Cancel)
    }

    fn keyboard_interactive(
        &self,
        name: String,
        instructions: String,
        prompts: Vec<KeyboardPrompt>,
    ) -> Option<Vec<String>> {
        self.ask(
            |reply| Prompt::Keyboard {
                name,
                instructions,
                prompts,
                reply,
            },
            None,
        )
    }

    fn passphrase(&self, key_path: String, failed: bool) -> Option<String> {
        self.ask(
            |reply| Prompt::Passphrase {
                key_path,
                failed,
                reply,
            },
            None,
        )
    }
}
