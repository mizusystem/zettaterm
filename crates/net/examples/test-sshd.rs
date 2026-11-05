//! 画面確認用のテスト SSH サーバー（127.0.0.1 のみ）。ホスト鍵は起動のたびに作る。
//!
//!     cargo run -p mt-net --example test-sshd -- 2222
//!
//! 受け付ける認証：ユーザー kt のパスワード `secret`、キーボードインタラクティブの `1234`。
//! 公開鍵はすべて断る（クライアントの自動認証が失敗して、認証ダイアログに進むのを確かめるため）。
//! シェルは入力をそのまま「echo:」付きで返し、`exit` で閉じる。

use std::sync::Arc;
use std::time::Duration;

use russh::keys::ssh_key::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Auth, Msg, Response};
use russh::{Channel, ChannelId, MethodKind, MethodSet};

#[derive(Clone, Default)]
struct Server {
    line: String,
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        let m: &[MethodKind] = &[
            MethodKind::Password,
            MethodKind::KeyboardInteractive,
            MethodKind::PublicKey,
        ];
        Ok(Auth::Reject {
            proceed_with_methods: Some(MethodSet::from(m)),
            partial_success: false,
        })
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        println!(
            "password auth: user={user} ok={}",
            user == "kt" && password == "secret"
        );
        Ok(if user == "kt" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        _: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        println!("public key offered by {user}: rejected");
        Ok(Auth::reject())
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        match response {
            None => Ok(Auth::Partial {
                name: "One-time code".into(),
                instructions: "Enter the 4-digit code (test: 1234).".into(),
                prompts: vec![("Code: ".into(), false)].into(),
            }),
            Some(mut r) => {
                let ok = r.next().is_some_and(|b| &b[..] == b"1234");
                println!("keyboard-interactive: user={user} ok={ok}");
                Ok(if ok { Auth::Accept } else { Auth::reject() })
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(
            channel,
            &b"Welcome to the Multi-Term test SSH server\r\n$ "[..],
        )?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        for &b in data {
            match b {
                b'\r' => {
                    let line = std::mem::take(&mut self.line);
                    if line == "exit" {
                        session.close(channel)?;
                        return Ok(());
                    }
                    session.data(channel, format!("\r\necho:{line}\r\n$ ").into_bytes())?;
                }
                0x7f => {
                    if self.line.pop().is_some() {
                        session.data(channel, &b"\x08 \x08"[..])?;
                    }
                }
                _ => {
                    self.line.push(b as char);
                    session.data(channel, vec![b])?;
                }
            }
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(
            channel,
            format!("\r\n[size {cols}x{rows}]\r\n$ ").into_bytes(),
        )?;
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(2222);
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    println!(
        "host key: {}",
        key.public_key().fingerprint(HashAlg::Sha256)
    );
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(100),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    println!("listening on 127.0.0.1:{port}");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            if let Ok(session) = server::run_stream(config, stream, Server::default()).await {
                let _ = session.await;
            }
        });
    }
}
