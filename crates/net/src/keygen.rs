//! SSH 鍵の生成（TTSSH の「SSH 鍵生成」）。OpenSSH の形式で秘密鍵と公開鍵を書く。

use std::io;
use std::path::{Path, PathBuf};

use russh::keys::ssh_key::{self, Algorithm, EcdsaCurve, LineEnding, PrivateKey};

/// 鍵の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    /// RSA（ビット数。2048 以上）
    Rsa(usize),
}

impl KeyKind {
    /// ファイル名の既定（`id_ed25519` など）。
    pub fn default_file_name(self) -> &'static str {
        match self {
            KeyKind::Ed25519 => "id_ed25519",
            KeyKind::EcdsaP256 | KeyKind::EcdsaP384 | KeyKind::EcdsaP521 => "id_ecdsa",
            KeyKind::Rsa(_) => "id_rsa",
        }
    }
}

/// 生成した鍵（OpenSSH の形式の文字列）。
#[derive(Clone)]
pub struct GeneratedKey {
    /// 秘密鍵（`-----BEGIN OPENSSH PRIVATE KEY-----`）。パスフレーズがあれば暗号化してある
    pub private: String,
    /// 公開鍵の 1 行（`ssh-ed25519 AAAA... comment`）。authorized_keys に足す
    pub public: String,
    /// SHA256 の指紋
    pub fingerprint: String,
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// 鍵を作る。`passphrase` が空でなければ秘密鍵を暗号化する。
pub fn generate(kind: KeyKind, comment: &str, passphrase: &str) -> io::Result<GeneratedKey> {
    let algorithm = match kind {
        KeyKind::Ed25519 => Algorithm::Ed25519,
        KeyKind::EcdsaP256 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        },
        KeyKind::EcdsaP384 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        },
        KeyKind::EcdsaP521 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        },
        KeyKind::Rsa(bits) => {
            if bits < 2048 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "RSA keys need at least 2048 bits",
                ));
            }
            Algorithm::Rsa { hash: None }
        }
    };
    let mut rng = rand::rng();
    let mut key = match kind {
        KeyKind::Rsa(bits) => {
            let rsa = ssh_key::private::RsaKeypair::random(&mut rng, bits).map_err(other)?;
            PrivateKey::from(rsa)
        }
        _ => PrivateKey::random(&mut rng, algorithm).map_err(other)?,
    };
    key.set_comment(comment);
    let fingerprint = key
        .public_key()
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string();
    let public = key.public_key().to_openssh().map_err(other)?;
    let key = if passphrase.is_empty() {
        key
    } else {
        key.encrypt(&mut rng, passphrase).map_err(other)?
    };
    let private = key.to_openssh(LineEnding::LF).map_err(other)?.to_string();
    Ok(GeneratedKey {
        private,
        public,
        fingerprint,
    })
}

/// `path`（秘密鍵）と `path.pub`（公開鍵）に書く。秘密鍵は本人だけが読めるようにする。
/// どちらかが既にあれば上書きせずに失敗する。
pub fn save(key: &GeneratedKey, path: &Path) -> io::Result<PathBuf> {
    let public_path = PathBuf::from(format!("{}.pub", path.display()));
    for p in [path, public_path.as_path()] {
        if p.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", p.display()),
            ));
        }
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    write_private(path, key.private.as_bytes())?;
    std::fs::write(&public_path, format!("{}\n", key.public))?;
    Ok(public_path)
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ed25519_with_passphrase_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate(KeyKind::Ed25519, "kt@test", "secret").unwrap();
        assert!(key.public.starts_with("ssh-ed25519 "));
        assert!(key.public.ends_with(" kt@test"));
        assert!(key.fingerprint.starts_with("SHA256:"));
        let path = dir.path().join("id_ed25519");
        let pub_path = save(&key, &path).unwrap();
        assert!(pub_path.ends_with("id_ed25519.pub"));
        // パスフレーズが無いと読めず、あれば読める
        assert!(russh::keys::load_secret_key(&path, None).is_err());
        let loaded = russh::keys::load_secret_key(&path, Some("secret")).unwrap();
        assert_eq!(loaded.public_key().to_openssh().unwrap(), key.public);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // 上書きはしない
        assert!(save(&key, &path).is_err());
    }

    #[test]
    fn ecdsa_without_passphrase() {
        let key = generate(KeyKind::EcdsaP256, "", "").unwrap();
        assert!(key.public.starts_with("ecdsa-sha2-nistp256 "));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k");
        save(&key, &path).unwrap();
        assert!(russh::keys::load_secret_key(&path, None).is_ok());
        assert!(generate(KeyKind::Rsa(1024), "", "").is_err());
    }
}
