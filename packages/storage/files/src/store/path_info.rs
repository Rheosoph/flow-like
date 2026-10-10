use super::{FlowLikeStore, local_store::LocalObjectStore};
use anyhow::{Context, Result};
use futures::StreamExt;
use object_store::{ObjectStoreExt, path::Path};
use std::{error::Error as _, fs::Metadata, io::ErrorKind};

fn is_missing(error: &object_store::Error) -> bool {
    if matches!(error, object_store::Error::NotFound { .. }) {
        return true;
    }

    // Local stores wrap errors for paths whose ancestor is a file as generic errors.
    let mut source = error.source();
    while let Some(error) = source {
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == ErrorKind::NotADirectory)
        {
            return true;
        }
        source = error.source();
    }
    false
}

async fn local_metadata(store: &LocalObjectStore, path: &Path) -> Result<Option<Metadata>> {
    let local_path = store.directory_to_filesystem(path)?;
    match tokio::fs::metadata(&local_path).await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(None)
        }
        Err(error) => Err(error).with_context(|| format!("Failed to inspect {path} in {store}")),
    }
}

impl FlowLikeStore {
    /// Checks for a regular local file or an object at this exact key.
    /// Missing paths return false; other storage errors are returned to the caller.
    pub async fn is_file(&self, path: &Path) -> Result<bool> {
        if let Self::Local(store) = self {
            return Ok(local_metadata(store, path)
                .await?
                .is_some_and(|metadata| metadata.is_file()));
        }

        let Some(parent) = path.parent() else {
            return Ok(false);
        };

        let store = self.as_generic();
        if matches!(self, Self::Other(_)) {
            // Wrapped local stores can create missing parents during head requests.
            // Listing also works for stores that expose native directories, such as SMB.
            return match store.list_with_delimiter(Some(&parent)).await {
                Ok(list) => Ok(list.objects.iter().any(|object| object.location == *path)),
                Err(error) if is_missing(&error) => Ok(false),
                Err(error) => {
                    Err(error).with_context(|| format!("Failed to inspect {path} in {store}"))
                }
            };
        }

        match store.head(path).await {
            Ok(_) => Ok(true),
            Err(error) if is_missing(&error) => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("Failed to inspect {path} in {store}"))
            }
        }
    }

    /// Checks for a local directory or a folder prefix reported by the object store.
    /// Object store roots are folders, including when they contain no objects.
    /// A key can be both an object and a folder prefix in an object store.
    pub async fn is_folder(&self, path: &Path) -> Result<bool> {
        if let Self::Local(store) = self {
            return Ok(local_metadata(store, path)
                .await?
                .is_some_and(|metadata| metadata.is_dir()));
        }

        let Some(parent) = path.parent() else {
            return Ok(true);
        };

        let store = self.as_generic();
        let generic = matches!(self, Self::Other(_));
        let mut objects = store.list(Some(path));
        while let Some(object) = objects.next().await {
            match object {
                // Cloud listings exclude the exact key, but can return a folder
                // marker at that key. Generic stores such as SMB also list files.
                Ok(object) if !generic || object.location != *path => return Ok(true),
                Ok(_) => {}
                Err(error) if is_missing(&error) => break,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to inspect {path} in {store}"));
                }
            }
        }
        if !generic {
            return Ok(false);
        }

        // Parent listings retain empty native directories behind generic store wrappers.
        match store.list_with_delimiter(Some(&parent)).await {
            Ok(list) => Ok(list.common_prefixes.contains(path)),
            Err(error) if is_missing(&error) => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("Failed to inspect {path} in {store}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::{PutPayload, memory::InMemory};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn local_store() -> (TempDir, FlowLikeStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::new(directory.path().to_path_buf()).unwrap();
        (directory, FlowLikeStore::Local(Arc::new(store)))
    }

    #[tokio::test]
    async fn local_paths_distinguish_files_empty_folders_and_missing_paths() {
        let (directory, store) = local_store();
        std::fs::write(directory.path().join("file.txt"), b"contents").unwrap();
        std::fs::create_dir(directory.path().join("empty")).unwrap();
        std::fs::create_dir(directory.path().join("build#123")).unwrap();

        for (path, is_file, is_folder) in [
            ("", false, true),
            ("file.txt", true, false),
            ("empty", false, true),
            ("build#123", false, true),
            ("missing", false, false),
            ("missing/child.txt", false, false),
            ("file.txt/child", false, false),
        ] {
            let path = Path::parse(path).unwrap();
            assert_eq!(store.is_file(&path).await.unwrap(), is_file, "{path}");
            assert_eq!(store.is_folder(&path).await.unwrap(), is_folder, "{path}");
        }

        assert!(!directory.path().join("missing").exists());
    }

    #[tokio::test]
    async fn object_paths_allow_files_and_virtual_folders_at_the_same_key() {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let root = Path::default();
        assert!(!store.is_file(&root).await.unwrap());
        assert!(store.is_folder(&root).await.unwrap());

        for key in [
            "file.txt",
            "folder/nested/file.txt",
            "both",
            "both/file.txt",
            "sibling-extra/file.txt",
        ] {
            store
                .as_generic()
                .put(&Path::from(key), PutPayload::from_static(b"contents"))
                .await
                .unwrap();
        }

        for store in [store.clone(), store.read_only()] {
            for (path, is_file, is_folder) in [
                ("file.txt", true, false),
                ("folder", false, true),
                ("folder/nested", false, true),
                ("folder/nested/file.txt", true, false),
                ("both", true, true),
                ("sibling", false, false),
                ("sibling-extra", false, true),
                ("missing", false, false),
                ("missing/child", false, false),
            ] {
                let path = Path::from(path);
                assert_eq!(store.is_file(&path).await.unwrap(), is_file, "{path}");
                assert_eq!(store.is_folder(&path).await.unwrap(), is_folder, "{path}");
            }
        }
    }

    #[tokio::test]
    async fn encoded_paths_keep_their_parent_and_filename() {
        let (_directory, local) = local_store();
        let memory = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let raw_folder = "Übersicht 100%";
        let raw_file = "report#2024.txt";

        let folder = Path::from(raw_folder);
        let file = folder.clone().join(raw_file);
        for store in [&local, &memory] {
            store
                .as_generic()
                .put(&file, PutPayload::from_static(b"contents"))
                .await
                .unwrap();
        }

        for store in [local.clone(), local.read_only(), memory] {
            let folder = Path::parse(folder.as_ref()).unwrap();
            let file = Path::parse(file.as_ref()).unwrap();
            assert!(store.is_folder(&folder).await.unwrap());
            assert!(!store.is_file(&folder).await.unwrap());
            assert!(store.is_file(&file).await.unwrap());
            assert!(!store.is_folder(&file).await.unwrap());
        }
    }

    #[tokio::test]
    async fn read_only_local_paths_preserve_empty_folders_without_creating_missing_parents() {
        let (directory, local) = local_store();
        let store = local.read_only();
        std::fs::write(directory.path().join("file.txt"), b"contents").unwrap();
        std::fs::create_dir(directory.path().join("empty")).unwrap();

        for (path, is_file, is_folder) in [
            ("", false, true),
            ("file.txt", true, false),
            ("empty", false, true),
            ("missing/child.txt", false, false),
            ("file.txt/child", false, false),
        ] {
            let path = Path::from(path);
            assert_eq!(store.is_file(&path).await.unwrap(), is_file, "{path}");
            assert_eq!(store.is_folder(&path).await.unwrap(), is_folder, "{path}");
        }

        assert!(!directory.path().join("missing").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn storage_errors_are_not_reported_as_missing_paths() {
        let (directory, local) = local_store();
        std::os::unix::fs::symlink("loop", directory.path().join("loop")).unwrap();
        let path = Path::from("loop");

        for store in [local.clone(), local.read_only()] {
            assert!(store.is_file(&path).await.is_err());
            assert!(store.is_folder(&path).await.is_err());
        }
    }
}
