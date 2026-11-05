//! ファイル転送（mt-xfer）を Session の ByteDivert に合わせ、UI から進み具合を見られるようにする。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mt_core as core;
use mt_xfer::{Direction, Protocol, Status, Transfer};

use crate::MtError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TransferProtocol {
    XModem { crc: bool, one_k: bool },
    YModem,
    ZModem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TransferStatus {
    Running,
    Done,
    Canceled,
    Failed,
}

/// 転送ダイアログに出す内容（Tera Term の「ファイル名」「転送済容量」）。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TransferProgress {
    pub status: TransferStatus,
    /// 失敗したときの理由
    pub message: Option<String>,
    pub file: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    pub files_done: u32,
    /// 受け取ったファイルの保存先
    pub saved: Vec<String>,
}

type Shared = Arc<Mutex<Box<dyn Transfer>>>;

/// 進行中の転送。UI はこれで進み具合を見る（中止は Session::cancel_transfer）。
#[derive(uniffi::Object)]
pub struct TransferHandle {
    transfer: Shared,
}

#[uniffi::export]
impl TransferHandle {
    pub fn progress(&self) -> TransferProgress {
        let t = self.transfer.lock().unwrap_or_else(|e| e.into_inner());
        let p = t.progress();
        let (status, message) = match t.status() {
            Status::Running => (TransferStatus::Running, None),
            Status::Done => (TransferStatus::Done, None),
            Status::Canceled => (TransferStatus::Canceled, None),
            Status::Failed(m) => (TransferStatus::Failed, Some(m)),
        };
        TransferProgress {
            status,
            message,
            file: p.file,
            bytes: p.bytes,
            total: p.total,
            files_done: p.files_done as u32,
            saved: p
                .saved
                .into_iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect(),
        }
    }
}

struct Divert(Shared);

impl core::ByteDivert for Divert {
    fn start(&mut self) -> Vec<u8> {
        self.lock().start(Instant::now())
    }
    fn receive(&mut self, data: &[u8]) -> Vec<u8> {
        self.lock().receive(data, Instant::now())
    }
    fn tick(&mut self) -> Vec<u8> {
        self.lock().tick(Instant::now())
    }
    fn cancel(&mut self) -> Vec<u8> {
        self.lock().cancel()
    }
    fn finished(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status()
            .is_finished()
    }
}

impl Divert {
    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn Transfer>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) fn create(
    protocol: TransferProtocol,
    send: bool,
    files: Vec<String>,
    destination: Option<String>,
) -> Result<(Arc<TransferHandle>, Box<dyn core::ByteDivert>), MtError> {
    let protocol = match protocol {
        TransferProtocol::XModem { crc, one_k } => Protocol::XModem { crc, one_k },
        TransferProtocol::YModem => Protocol::YModem,
        TransferProtocol::ZModem => Protocol::ZModem,
    };
    let direction = if send {
        Direction::Send(files.into_iter().map(PathBuf::from).collect())
    } else {
        let dest = destination.ok_or_else(|| MtError::Io {
            message: "no destination".into(),
        })?;
        Direction::Receive(PathBuf::from(dest))
    };
    let transfer: Shared = Arc::new(Mutex::new(mt_xfer::new_transfer(protocol, direction)?));
    let handle = Arc::new(TransferHandle {
        transfer: Arc::clone(&transfer),
    });
    Ok((handle, Box::new(Divert(transfer))))
}
