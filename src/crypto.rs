use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, Generate, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hkdf::Hkdf;
use rand::RngExt;
use serde_json::Value;
use sha2::Sha256;
use std::path::{Path, PathBuf};

use crate::config::{Config, Paths};

pub const ENC_PREFIX: &str = "enc:v1:";

pub struct Keyring {
    master: [u8; 32],
}

impl Keyring {
    pub fn from_master(master: [u8; 32]) -> Self {
        Self { master }
    }

    pub fn load(paths: &Paths, config: &Config) -> anyhow::Result<Self> {
        if let Ok(value) = std::env::var("LUNADB_MASTER_KEY") {
            let master = parse_master_key(value.trim()).context("invalid LUNADB_MASTER_KEY")?;
            return Ok(Self { master });
        }
        let path = master_key_path(paths, config);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read master key at {}", path.display()))?;
        let master = parse_master_key(raw.trim())
            .with_context(|| format!("invalid master key at {}", path.display()))?;
        Ok(Self { master })
    }

    pub fn db_key(&self, db: &str) -> [u8; 32] {
        let hkdf = Hkdf::<Sha256>::new(Some(b"lunadb:v1"), &self.master);
        let mut key = [0u8; 32];
        hkdf.expand(format!("db:{db}").as_bytes(), &mut key)
            .expect("valid hkdf output size");
        key
    }

    pub fn encrypt_bytes(&self, db: &str, plaintext: &[u8]) -> anyhow::Result<String> {
        let key = Key::<Aes256Gcm>::from(self.db_key(db));
        let cipher = Aes256Gcm::new(&key);
        let nonce = Nonce::<U12>::generate();
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| anyhow::anyhow!("failed to encrypt"))?;
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(format!("{ENC_PREFIX}{}", STANDARD.encode(blob)))
    }

    pub fn decrypt_bytes(&self, db: &str, token: &str) -> anyhow::Result<Vec<u8>> {
        let encoded = token
            .strip_prefix(ENC_PREFIX)
            .context("value is not in enc:v1 format")?;
        let blob = STANDARD.decode(encoded).context("invalid base64")?;
        if blob.len() < 12 {
            bail!("invalid ciphertext");
        }
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let nonce: &Nonce<U12> = nonce_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid nonce"))?;
        let key = Key::<Aes256Gcm>::from(self.db_key(db));
        let cipher = Aes256Gcm::new(&key);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("failed to decrypt (wrong key or corrupted data)"))
    }

    pub fn encrypt_value(&self, db: &str, value: &Value) -> anyhow::Result<Value> {
        let bytes = serde_json::to_vec(value)?;
        Ok(Value::String(self.encrypt_bytes(db, &bytes)?))
    }

    pub fn decrypt_value(&self, db: &str, token: &str) -> anyhow::Result<Value> {
        let bytes = self.decrypt_bytes(db, token)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

pub fn is_encrypted_value(value: &Value) -> bool {
    is_encrypted_str(value.as_str().unwrap_or_default())
}

pub fn is_encrypted_str(value: &str) -> bool {
    value.starts_with(ENC_PREFIX)
}

pub fn master_key_path(paths: &Paths, config: &Config) -> PathBuf {
    match &config.encryption.master_key_file {
        Some(path) => crate::config::expand_home(path),
        None => paths.meta.join("master.key"),
    }
}

pub fn parse_master_key(raw: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = hex::decode(raw).context("invalid hex")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("master key must be 32 bytes (64 hex)"))
}

pub fn write_master_key(path: &Path) -> anyhow::Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", hex::encode(bytes)))?;
    set_private_permissions(path)?;
    Ok(bytes)
}

#[cfg(unix)]
pub fn set_private_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_private_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub fn has_private_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o077 == 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn has_private_permissions(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let keyring = Keyring::from_master([1u8; 32]);
        let value = json!({"secret": "value", "number": 42, "list": [1, 2, 3]});
        let token = keyring.encrypt_value("app", &value).unwrap();
        let token_str = token.as_str().unwrap();
        assert!(token_str.starts_with(ENC_PREFIX));
        assert!(is_encrypted_str(token_str));
        assert!(is_encrypted_value(&token));
        let decrypted = keyring.decrypt_value("app", token_str).unwrap();
        assert_eq!(decrypted, value);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let token = Keyring::from_master([1u8; 32])
            .encrypt_value("app", &json!("secret"))
            .unwrap();
        let token = token.as_str().unwrap().to_string();
        let other = Keyring::from_master([2u8; 32]);
        assert!(other.decrypt_value("app", &token).is_err());
    }

    #[test]
    fn keys_are_isolated_per_db() {
        let keyring = Keyring::from_master([3u8; 32]);
        assert_ne!(keyring.db_key("a"), keyring.db_key("b"));
        let token = keyring.encrypt_value("a", &json!("x")).unwrap();
        let token = token.as_str().unwrap().to_string();
        assert!(keyring.decrypt_value("b", &token).is_err());
        assert!(keyring.decrypt_value("a", &token).is_ok());
    }

    #[test]
    fn nonce_is_unique_per_encryption() {
        let keyring = Keyring::from_master([4u8; 32]);
        let first = keyring.encrypt_value("app", &json!("x")).unwrap();
        let second = keyring.encrypt_value("app", &json!("x")).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn invalid_token_errors() {
        let keyring = Keyring::from_master([5u8; 32]);
        assert!(keyring.decrypt_value("app", "plain-text").is_err());
        assert!(keyring.decrypt_value("app", "enc:v1:bm90YQ==").is_err());
    }

    #[test]
    fn parse_master_key_validates_length() {
        assert!(parse_master_key(&"ab".repeat(32)).is_ok());
        assert!(parse_master_key("abc").is_err());
        assert!(parse_master_key(&"zz".repeat(32)).is_err());
    }
}
