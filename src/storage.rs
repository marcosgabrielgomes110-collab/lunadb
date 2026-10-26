use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Msgpack,
}

impl Format {
    pub fn from_name(name: &str) -> Self {
        match name {
            "json" => Format::Json,
            _ => Format::Msgpack,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Msgpack => "msgpack",
        }
    }
}

pub fn format_of(path: &Path) -> Format {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => Format::Json,
        _ => Format::Msgpack,
    }
}

pub fn encode<T: Serialize>(format: Format, value: &T) -> anyhow::Result<Vec<u8>> {
    Ok(match format {
        Format::Json => serde_json::to_vec_pretty(value)?,
        Format::Msgpack => rmp_serde::to_vec_named(value)?,
    })
}

pub fn decode<T: DeserializeOwned>(format: Format, bytes: &[u8]) -> anyhow::Result<T> {
    Ok(match format {
        Format::Json => serde_json::from_slice(bytes)?,
        Format::Msgpack => rmp_serde::from_slice(bytes)?,
    })
}

pub async fn read_value<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(decode(format_of(path), &bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}

pub async fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    tokio::fs::create_dir_all(parent).await?;

    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
    }

    tokio::fs::rename(&tmp, path).await?;

    // Fsync the parent directory so directory metadata changes are durable
    if let Ok(dir) = tokio::fs::File::open(parent).await {
        let _ = dir.sync_all().await;
    }

    Ok(())
}

pub async fn write_value<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let body = encode(format_of(path), value)?;
    atomic_write_bytes(path, &body).await
}

pub async fn read_json<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}

pub async fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    atomic_write_bytes(path, &body).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn msgpack_and_json_roundtrip() {
        let value = json!({
            "name": "users",
            "records": {"abc": {"name": "Ana", "tags": ["a", "b"], "ok": true}},
        });
        for format in [Format::Msgpack, Format::Json] {
            let encoded = encode(format, &value).unwrap();
            let decoded: serde_json::Value = decode(format, &encoded).unwrap();
            assert_eq!(decoded, value, "roundtrip failed for {format:?}");
        }
    }

    #[test]
    fn format_of_by_extension() {
        assert_eq!(format_of(Path::new("users.json")), Format::Json);
        assert_eq!(format_of(Path::new("users.msgpack")), Format::Msgpack);
        assert_eq!(format_of(Path::new("users")), Format::Msgpack);
        assert_eq!(Format::from_name("json").extension(), "json");
        assert_eq!(Format::from_name("anything").extension(), "msgpack");
    }
}
