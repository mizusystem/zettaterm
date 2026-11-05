//! テスト用 SSH サーバーの exec。`scp` のコマンドだけを本物の（OpenSSH の）scp で動かし、
//! チャネルの入出力をつなぐ。ssh_client テストと examples/test-sshd で共有する。

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use russh::ChannelId;
use russh::server::Handle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct Execs(Arc<Mutex<HashMap<ChannelId, ChildStdin>>>);

impl Execs {
    /// `command` が scp なら `dir` で起動して true を返す。シェルの特殊文字を含むものは断る。
    pub async fn spawn_scp(
        &self,
        command: &str,
        dir: &Path,
        channel: ChannelId,
        handle: Handle,
    ) -> bool {
        if !command.starts_with("scp ") || command.contains(|c| ";&|`$<>\n".contains(c)) {
            return false;
        }
        let Ok(mut child) = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        self.0.lock().await.insert(channel, stdin);
        let execs = self.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if handle.data(channel, buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(1);
            execs.0.lock().await.remove(&channel);
            let _ = handle.exit_status_request(channel, code as u32).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
        true
    }

    /// exec のチャネルなら、受け取ったデータを scp の標準入力へ渡して true を返す。
    pub async fn write(&self, channel: ChannelId, data: &[u8]) -> bool {
        match self.0.lock().await.get_mut(&channel) {
            Some(stdin) => {
                let _ = stdin.write_all(data).await;
                true
            }
            None => false,
        }
    }

    /// クライアントが送り終えた（標準入力を閉じる）。
    pub async fn eof(&self, channel: ChannelId) {
        self.0.lock().await.remove(&channel);
    }
}
