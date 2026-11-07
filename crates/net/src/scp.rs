//! SCP（Tera Term / TTSSH の「SSH SCP」）。ログイン済みの SSH 接続の上に別のチャネルを開き、
//! 相手の `scp -t`（受け取り）/ `scp -f`（送り出し）と話す。端末はそのまま使える。
//!
//! フォルダは -r で中身ごと（`D0755 0 名前` … `E`）。
//!
//! 手順（古典的な rcp の手順）
//! - 送る：`scp -t 先` → 0 を待つ → `C0644 大きさ 名前\n` → 0 → 中身 → `\0` → 0 → … → EOF
//! - 受け取る：`scp -f パス` → `\0` を送る → `C…\n` を読む → `\0` → 中身と `\0` を読む → `\0` → … → EOF
//!
//! 応答の 1 バイト目：0 は成功、1 は警告、2 は失敗（どちらも続く 1 行が理由）。

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::{Handle, Msg};
use russh::{Channel, ChannelMsg};
use tokio::runtime;

use crate::ssh_client::Client;

const CHUNK: usize = 32 * 1024;
/// 制御行の長さの上限
const MAX_LINE: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScpStatus {
    Running,
    Done,
    Canceled,
    Failed(String),
}

/// 進み具合（TTSSH の「SCP ファイル送信中 / 受信中」）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScpProgress {
    pub status: ScpStatus,
    pub file: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    pub files_done: usize,
    /// 受け取ったファイルの保存先
    pub saved: Vec<PathBuf>,
}

impl Default for ScpProgress {
    fn default() -> Self {
        Self {
            status: ScpStatus::Running,
            file: None,
            bytes: 0,
            total: None,
            files_done: 0,
            saved: Vec::new(),
        }
    }
}

/// 進行中の SCP。
#[derive(Clone)]
pub struct ScpJob {
    progress: Arc<Mutex<ScpProgress>>,
    cancel: Arc<AtomicBool>,
}

impl ScpJob {
    pub fn progress(&self) -> ScpProgress {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// 終わるまで待つ（マクロの scpsend / scprecv）。`stopped` が true を返したら取り消す。
    pub fn wait(&self, stopped: &dyn Fn() -> bool) -> Result<ScpProgress, String> {
        loop {
            let p = self.progress();
            match &p.status {
                ScpStatus::Running => {}
                ScpStatus::Done => return Ok(p),
                ScpStatus::Canceled => return Err("canceled".into()),
                ScpStatus::Failed(e) => return Err(e.clone()),
            }
            if stopped() {
                self.cancel();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    fn update(&self, f: impl FnOnce(&mut ScpProgress)) {
        f(&mut self.progress.lock().unwrap_or_else(|e| e.into_inner()));
    }

    fn canceled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// SSH 接続でファイルをやり取りする窓口。
#[derive(Clone)]
pub struct SshFiles {
    handle: Arc<Handle<Client>>,
    rt: runtime::Handle,
}

impl SshFiles {
    pub(crate) fn new(handle: Arc<Handle<Client>>, rt: runtime::Handle) -> Self {
        Self { handle, rt }
    }

    /// ローカルの `files` をリモートの `remote_dir`（空ならホーム）へ送る。
    pub fn upload(&self, files: Vec<PathBuf>, remote_dir: &str) -> ScpJob {
        let job = ScpJob {
            progress: Arc::default(),
            cancel: Arc::default(),
        };
        let (handle, j) = (Arc::clone(&self.handle), job.clone());
        let dest = if remote_dir.trim().is_empty() {
            ".".to_owned()
        } else {
            remote_dir.trim().to_owned()
        };
        self.rt.spawn(async move {
            let result = upload(&handle, &files, &dest, &j).await;
            finish(&j, result);
        });
        job
    }

    /// ファイル 1 つを送り、終わるまで待つ。`remote` はリモートのファイル名かフォルダ（空ならホーム）。
    pub fn send_file(
        &self,
        local: &Path,
        remote: &str,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        self.upload(vec![local.to_owned()], remote)
            .wait(stopped)
            .map(|_| ())
    }

    /// リモートのファイル 1 つを `local`（ファイル名）として受け取り、終わるまで待つ。
    /// `local` が既にあるフォルダ（または `/` で終わる）なら、リモートの名前のままそこへ受け取る。
    pub fn recv_file(
        &self,
        remote: &str,
        local: &Path,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let as_dir = local.is_dir() || local.as_os_str().to_string_lossy().ends_with('/');
        if as_dir {
            std::fs::create_dir_all(local).map_err(|e| e.to_string())?;
            return self
                .download(remote, local.to_path_buf())
                .wait(stopped)
                .map(|_| ());
        }
        let dir = local
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let progress = self.download(remote, dir).wait(stopped)?;
        // 受け取り先は同じ名前のファイルを上書きしないので、指定の名前に付け替える
        if let Some(saved) = progress.saved.first()
            && saved != local
        {
            std::fs::rename(saved, local).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// リモートの `remote_path` をローカルの `local_dir` へ受け取る。
    pub fn download(&self, remote_path: &str, local_dir: PathBuf) -> ScpJob {
        let job = ScpJob {
            progress: Arc::default(),
            cancel: Arc::default(),
        };
        let (handle, j) = (Arc::clone(&self.handle), job.clone());
        let path = remote_path.trim().to_owned();
        self.rt.spawn(async move {
            let result = download(&handle, &path, &local_dir, &j).await;
            finish(&j, result);
        });
        job
    }
}

fn finish(job: &ScpJob, result: io::Result<()>) {
    job.update(|p| {
        p.status = match result {
            Ok(()) => ScpStatus::Done,
            Err(_) if job.canceled() => ScpStatus::Canceled,
            Err(e) => ScpStatus::Failed(e.to_string()),
        }
    });
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// ローカルのファイルの失敗に、どのファイルかを書き足す。
fn with_path(e: io::Error, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

/// リモートのシェルに渡すため、単一引用符で囲む。
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// チャネルからの受信を、行やバイト数で読めるようにする。
struct ChannelIn {
    channel: Channel<Msg>,
    buf: Vec<u8>,
    eof: bool,
}

impl ChannelIn {
    async fn fill(&mut self) -> io::Result<()> {
        while !self.eof {
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    self.buf.extend_from_slice(&data);
                    return Ok(());
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => self.eof = true,
                _ => {}
            }
        }
        Err(io::Error::from(io::ErrorKind::UnexpectedEof))
    }

    async fn byte(&mut self) -> io::Result<u8> {
        while self.buf.is_empty() {
            self.fill().await?;
        }
        Ok(self.buf.remove(0))
    }

    async fn line(&mut self) -> io::Result<String> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                return Ok(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
            }
            // 制御行（C… / D… / エラー文）は短い。改行が来ないまま長くなったら壊れた相手とみなす
            if self.buf.len() > MAX_LINE {
                return Err(other("scp: control line too long"));
            }
            self.fill().await?;
        }
    }

    /// 相手の応答（0 / 1 / 2）を読む。
    async fn ack(&mut self) -> io::Result<()> {
        match self.byte().await? {
            0 => Ok(()),
            1 | 2 => Err(other(format!("scp: {}", self.line().await?))),
            b => Err(other(format!("scp: unexpected reply {b:#x}"))),
        }
    }

    /// 受信が残っていないか、相手が閉じたかを見る（受け取りの終わりの判定）。
    async fn at_end(&mut self) -> bool {
        if !self.buf.is_empty() {
            return false;
        }
        self.fill().await.is_err()
    }
}

async fn open_exec(handle: &Handle<Client>, command: String) -> io::Result<Channel<Msg>> {
    let channel = handle.channel_open_session().await.map_err(other)?;
    channel.exec(true, command).await.map_err(other)?;
    Ok(channel)
}

async fn upload(
    handle: &Handle<Client>,
    files: &[PathBuf],
    dest: &str,
    job: &ScpJob,
) -> io::Result<()> {
    // フォルダがあれば -r（中身ごと）。複数なら宛先はフォルダ（-d）
    let recursive = files.iter().any(|p| p.is_dir());
    let mut flags = String::new();
    if recursive {
        flags.push_str("-r ");
    }
    if files.len() > 1 {
        flags.push_str("-d ");
    }
    let channel = open_exec(handle, format!("scp {flags}-t -- {}", shell_quote(dest))).await?;
    let mut input = ChannelIn {
        channel,
        buf: Vec::new(),
        eof: false,
    };
    input.ack().await?;
    let mut visited = std::collections::HashSet::new();
    for path in files {
        send_entry(&mut input, path, job, &mut visited).await?;
    }
    let _ = input.channel.eof().await;
    let _ = input.channel.close().await;
    Ok(())
}

fn entry_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().replace('\n', " "))
        .unwrap_or_default()
}

/// ファイルか、フォルダを中身ごと送る（D … / 中身 / E）。
/// `visited` は送ったフォルダ（実体のパス）。シンボリックリンクの輪で同じフォルダを二度たどらない。
fn send_entry<'a>(
    input: &'a mut ChannelIn,
    path: &'a Path,
    job: &'a ScpJob,
    visited: &'a mut std::collections::HashSet<PathBuf>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'a>> {
    Box::pin(async move {
        if path.is_dir() {
            let real = std::fs::canonicalize(path).map_err(|e| with_path(e, path))?;
            if !visited.insert(real) {
                return Ok(());
            }
            let header = format!("D0755 0 {}\n", entry_name(path));
            input.channel.data(header.as_bytes()).await.map_err(other)?;
            input.ack().await?;
            let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
                .map_err(|e| with_path(e, path))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .collect();
            entries.sort();
            for entry in entries {
                send_entry(input, &entry, job, visited).await?;
            }
            input.channel.data(&b"E\n"[..]).await.map_err(other)?;
            return input.ack().await;
        }
        // 少しずつ読んで送る（巨大なファイルを丸ごとメモリに載せない）
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(|e| with_path(e, path))?;
        let size = file.metadata().map_err(|e| with_path(e, path))?.len();
        let name = entry_name(path);
        job.update(|p| {
            p.file = Some(name.clone());
            p.bytes = 0;
            p.total = Some(size);
        });
        let header = format!("C0644 {size} {name}\n");
        input.channel.data(header.as_bytes()).await.map_err(other)?;
        input.ack().await?;
        let mut left = size;
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            if job.canceled() {
                let _ = input.channel.close().await;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "canceled"));
            }
            let want = (left as usize).min(CHUNK);
            file.read_exact(&mut buf[..want])
                .map_err(|e| with_path(e, path))?;
            input.channel.data(&buf[..want]).await.map_err(other)?;
            left -= want as u64;
            job.update(|p| p.bytes += want as u64);
        }
        input.channel.data(&[0u8][..]).await.map_err(other)?;
        input.ack().await?;
        job.update(|p| p.files_done += 1);
        Ok(())
    })
}

/// 相手から来た名前のうち、フォルダの中に置ける部分（`a/../b` などは使わない）。
fn safe_name(name: &str) -> Option<String> {
    // `/` と `\` のどちらでも区切り、最後の要素だけを使う。制御文字は除く
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = last.chars().filter(|c| !c.is_control()).collect();
    Some(clean).filter(|n| !n.is_empty() && n != "." && n != "..")
}

async fn download(
    handle: &Handle<Client>,
    remote: &str,
    local_dir: &Path,
    job: &ScpJob,
) -> io::Result<()> {
    if remote.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "remote path is empty",
        ));
    }
    // -r ならファイルでもフォルダでも受け取れる
    let channel = open_exec(handle, format!("scp -r -f -- {}", shell_quote(remote))).await?;
    let mut input = ChannelIn {
        channel,
        buf: Vec::new(),
        eof: false,
    };
    // 今いるフォルダ（先頭は受け取り先）
    let mut dirs: Vec<PathBuf> = vec![local_dir.to_owned()];
    input.channel.data(&[0u8][..]).await.map_err(other)?;
    loop {
        if input.at_end().await {
            break;
        }
        let first = input.byte().await?;
        match first {
            b'C' | b'D' => {}
            // 時刻（-p）。使わないが、受け取ったら応答する
            b'T' => {
                input.line().await?;
                input.channel.data(&[0u8][..]).await.map_err(other)?;
                continue;
            }
            b'E' => {
                input.line().await?;
                if dirs.len() > 1 {
                    dirs.pop();
                }
                input.channel.data(&[0u8][..]).await.map_err(other)?;
                continue;
            }
            1 | 2 => return Err(other(format!("scp: {}", input.line().await?))),
            _ => return Err(other("scp: unexpected data")),
        }
        // Cmmmm size name / Dmmmm 0 name
        let line = input.line().await?;
        let mut parts = line.splitn(3, ' ');
        let _mode = parts.next();
        let size: u64 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| other("scp: bad header"))?;
        let name = safe_name(parts.next().unwrap_or("")).unwrap_or_else(|| "received".to_owned());
        let here = dirs.last().cloned().unwrap_or_else(|| local_dir.to_owned());
        let top = dirs.len() == 1;
        if first == b'D' {
            // いちばん上のフォルダは既存と重ならない名前にし、中は相手の名前のまま
            let dir = if top {
                unique_path(&here, &name)
            } else {
                here.join(&name)
            };
            std::fs::create_dir_all(&dir).map_err(|e| with_path(e, &dir))?;
            if top {
                job.update(|p| p.saved.push(dir.clone()));
            }
            dirs.push(dir);
            input.channel.data(&[0u8][..]).await.map_err(other)?;
            continue;
        }
        let path = if top {
            unique_path(&here, &name)
        } else {
            here.join(&name)
        };
        let mut file = std::fs::File::create(&path).map_err(|e| with_path(e, &path))?;
        job.update(|p| {
            p.file = Some(name.clone());
            p.bytes = 0;
            p.total = Some(size);
        });
        input.channel.data(&[0u8][..]).await.map_err(other)?;
        let mut left = size;
        while left > 0 {
            if job.canceled() {
                drop(file);
                let _ = std::fs::remove_file(&path);
                let _ = input.channel.close().await;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "canceled"));
            }
            if input.buf.is_empty() {
                input.fill().await?;
            }
            let n = (left as usize).min(input.buf.len());
            file.write_all(&input.buf[..n])?;
            input.buf.drain(..n);
            left -= n as u64;
            job.update(|p| p.bytes += n as u64);
        }
        file.sync_all()?;
        input.ack().await?;
        input.channel.data(&[0u8][..]).await.map_err(other)?;
        job.update(|p| {
            p.files_done += 1;
            if top {
                p.saved.push(path.clone());
            }
        });
    }
    let _ = input.channel.close().await;
    Ok(())
}

/// 既存のファイルと重ならない名前（`a.txt` → `a.1.txt`）。相手から来た名前のフォルダ部分は捨てる。
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    // 相手から来た名前は `/` と `\\` のどちらでも区切り、最後の要素だけを使う（制御文字は除く）
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let base: String = last.chars().filter(|c| !c.is_control()).collect();
    let base = Some(base)
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "received".to_owned());
    let candidate = dir.join(&base);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match base.rfind('.') {
        Some(i) if i > 0 => (&base[..i], &base[i..]),
        _ => (base.as_str(), ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem}.{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_names() {
        assert_eq!(shell_quote("my file's.txt"), r"'my file'\''s.txt'");
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "../x"), dir.path().join("x"));
        // 相手から来た名前は、`/` と `\` のどちらでも区切った最後だけを使う
        assert_eq!(safe_name("..\\..\\evil.txt").as_deref(), Some("evil.txt"));
        assert_eq!(safe_name("a/b/../c").as_deref(), Some("c"));
        assert_eq!(safe_name("bad\u{7}name").as_deref(), Some("badname"));
        assert_eq!(safe_name(".."), None);
        assert_eq!(safe_name(""), None);
    }
}
