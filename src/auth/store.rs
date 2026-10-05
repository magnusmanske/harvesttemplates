//! Sessions as one JSON file each, so logins survive restarts. Files hold OAuth
//! tokens, hence owner-only permissions. Expired files are removed when read.

use async_trait::async_trait;
use std::io::ErrorKind;
use std::path::PathBuf;
use tower_sessions::SessionStore;
use tower_sessions::session::{Id, Record};
use tower_sessions::session_store::{Error, Result};

#[derive(Debug, Clone)]
pub struct FileSessionStore {
    dir: PathBuf,
}

impl FileSessionStore {
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        restrict(&dir, 0o700)?;
        Ok(Self { dir })
    }

    fn path(&self, id: &Id) -> PathBuf {
        // Ids are URL-safe base64, so safe as file names.
        self.dir.join(format!("{id}.json"))
    }
}

#[cfg(unix)]
fn restrict(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_: &std::path::Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

fn backend(e: impl std::fmt::Display) -> Error {
    Error::Backend(e.to_string())
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn create(&self, record: &mut Record) -> Result<()> {
        while tokio::fs::try_exists(self.path(&record.id)).await.map_err(backend)? {
            record.id = Id::default();
        }
        self.save(record).await
    }

    async fn save(&self, record: &Record) -> Result<()> {
        let json = serde_json::to_vec(record).map_err(|e| Error::Encode(e.to_string()))?;
        let path = self.path(&record.id);
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, json).await.map_err(backend)?;
        restrict(&tmp, 0o600).map_err(backend)?;
        // Atomic replace: a concurrent load never sees a partial file.
        tokio::fs::rename(&tmp, &path).await.map_err(backend)
    }

    async fn load(&self, id: &Id) -> Result<Option<Record>> {
        let bytes = match tokio::fs::read(self.path(id)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(backend(e)),
        };
        let record: Record = serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))?;
        if record.expiry_date < time::OffsetDateTime::now_utc() {
            self.delete(id).await?;
            return Ok(None);
        }
        Ok(Some(record))
    }

    async fn delete(&self, id: &Id) -> Result<()> {
        match tokio::fs::remove_file(self.path(id)).await {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(backend(e)),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Duration, OffsetDateTime};

    fn record(expires_in: Duration) -> Record {
        Record { id: Id::default(), data: Default::default(), expiry_date: OffsetDateTime::now_utc() + expires_in }
    }

    #[tokio::test]
    async fn roundtrip_expiry_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSessionStore::new(dir.path().join("s")).unwrap();
        let mut live = record(Duration::hours(1));
        store.create(&mut live).await.unwrap();
        assert_eq!(store.load(&live.id).await.unwrap().map(|r| r.id), Some(live.id));
        store.delete(&live.id).await.unwrap();
        assert!(store.load(&live.id).await.unwrap().is_none());

        let mut expired = record(Duration::minutes(-1));
        store.create(&mut expired).await.unwrap();
        assert!(store.load(&expired.id).await.unwrap().is_none());
        assert!(!store.path(&expired.id).exists(), "expired file is removed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn permissions_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = FileSessionStore::new(dir.path().join("s")).unwrap();
        let mut rec = record(Duration::hours(1));
        store.create(&mut rec).await.unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join("s")), 0o700);
        assert_eq!(mode(&store.path(&rec.id)), 0o600);
    }
}
