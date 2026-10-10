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
    pub fn load(paths: &Paths, config: &Config) -> anyhow::Result<Self> {
        if let Ok(value) = std::env::var("LUNADB_MASTER_KEY") {
            let master = parse_master_key(value.trim()).context("LUNADB_MASTER_KEY invalida")?;
            return Ok(Self { master });
        }
        let path = master_key_path(paths, config);
        let raw = std::fs::read_to_string(&path).with_context(|| {
            format!("falha ao ler master key em {}", path.display())
        })?;
        let master = parse_master_key(raw.trim())
            .with_context(|| format!("master key invalida em {}", path.display()))?;
        Ok(Self { master })
    }

    pub fn db_key(&self, db: &str) -> [u8; 32] {
        let hkdf = Hkdf::<Sha256>::new(Some(b"lunadb:v1"), &self.master);
        let mut key = [0u8; 32];
        hkdf.expand(format!("db:{db}").as_bytes(), &mut key)
            .expect("tamanho de saida hkdf valido");
        key
    }

    pub fn encrypt_bytes(&self, db: &str, plaintext: &[u8]) -> anyhow::Result<String> {
        let key = Key::<Aes256Gcm>::from(self.db_key(db));
        let cipher = Aes256Gcm::new(&key);
        let nonce = Nonce::<U12>::generate();
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| anyhow::anyhow!("falha ao cifrar"))?;
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(format!("{ENC_PREFIX}{}", STANDARD.encode(blob)))
    }

    pub fn decrypt_bytes(&self, db: &str, token: &str) -> anyhow::Result<Vec<u8>> {
        let encoded = token
            .strip_prefix(ENC_PREFIX)
            .context("valor nao esta no formato enc:v1")?;
        let blob = STANDARD.decode(encoded).context("base64 invalido")?;
        if blob.len() < 12 {
            bail!("ciphertext invalido");
        }
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let nonce: &Nonce<U12> = nonce_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("nonce invalido"))?;
        let key = Key::<Aes256Gcm>::from(self.db_key(db));
        let cipher = Aes256Gcm::new(&key);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("falha ao decifrar (chave errada ou dado corrompido)"))
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
    let bytes = hex::decode(raw).context("hex invalido")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("master key deve ter 32 bytes (64 hex)"))
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
