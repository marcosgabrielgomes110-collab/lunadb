use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;
use tokio::io::AsyncWriteExt;

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

pub async fn write_value<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let body = encode(format_of(path), value)?;
    write_atomic(path, &body).await
}

/// Writes `body` to `path` atomically and durably: the data is flushed to the
/// temporary file before the rename, and the parent directory is synced after
/// it, so a crash cannot leave a torn file or lose the rename.
async fn write_atomic(path: &Path, body: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(body).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await?;
        sync_parent_dir(path).await?;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if result.is_err() {
        // A failed write must not leave a temp file behind. After a successful
        // rename the temp file no longer exists, so this is a no-op.
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result.map_err(Into::into)
}

#[cfg(unix)]
async fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    // fsync on a directory makes the rename durable, not just the file content.
    tokio::fs::File::open(parent).await?.sync_all().await
}

#[cfg(not(unix))]
async fn sync_parent_dir(_path: &Path) -> std::io::Result<()> {
    // Windows has no portable equivalent of fsync on a directory. The file
    // content itself was already synced before the rename.
    Ok(())
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
    write_atomic(path, &body).await
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
