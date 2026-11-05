//! SSH SCP（TTSSH の「SSH SCP」）。内蔵の SSH 接続の上でファイルをやり取りする。
//! 進み具合は転送ダイアログと同じ TransferProgress で返す。

use mt_net::{ScpJob, ScpStatus};

use crate::transfer::{TransferProgress, TransferStatus};

/// 進行中の SCP。
#[derive(uniffi::Object)]
pub struct ScpHandle {
    pub(crate) job: ScpJob,
}

#[uniffi::export]
impl ScpHandle {
    pub fn progress(&self) -> TransferProgress {
        let p = self.job.progress();
        let (status, message) = match p.status {
            ScpStatus::Running => (TransferStatus::Running, None),
            ScpStatus::Done => (TransferStatus::Done, None),
            ScpStatus::Canceled => (TransferStatus::Canceled, None),
            ScpStatus::Failed(m) => (TransferStatus::Failed, Some(m)),
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

    pub fn cancel(&self) {
        self.job.cancel();
    }
}
