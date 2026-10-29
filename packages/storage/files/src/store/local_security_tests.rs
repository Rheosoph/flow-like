#![cfg(unix)]

use super::local_store::LocalObjectStore;
use bytes::Bytes;
use futures::StreamExt;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload, path::Path};
use std::{fs, os::unix::fs::symlink};
use tempfile::TempDir;

const OUTSIDE: &[u8] = b"outside-grant-fixture";

#[tokio::test]
async fn fifo_reads_fail_without_waiting_for_an_external_writer() {
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("pipe");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let store = LocalObjectStore::new(root.path().into()).unwrap();
    for head in [false, true] {
        let options = object_store::GetOptions {
            head,
            ..Default::default()
        };
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            store.get_opts(&Path::from("pipe"), options),
        )
        .await;
        if read.is_err() {
            // Unblock an accidental blocking open so a failed regression does
            // not leave the test runtime waiting on its worker indefinitely.
            let path = fifo.clone();
            tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(path))
                .await
                .unwrap()
                .unwrap();
            panic!("A storage read waited for a FIFO writer");
        }
        assert!(read.unwrap().is_err());
    }
    assert!(store.list(None).collect::<Vec<_>>().await.is_empty());
}

struct Fixture {
    root: TempDir,
    outside: TempDir,
    store: LocalObjectStore,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("project")).unwrap();
        fs::write(root.path().join("project/ordinary.txt"), b"inside").unwrap();
        fs::write(outside.path().join("secret.txt"), OUTSIDE).unwrap();
        fs::write(outside.path().join("target.txt"), OUTSIDE).unwrap();
        symlink(outside.path(), root.path().join("project/link")).unwrap();
        symlink(
            outside.path().join("secret.txt"),
            root.path().join("project/file-link"),
        )
        .unwrap();
        let store = LocalObjectStore::new(root.path().to_path_buf()).unwrap();
        Self {
            root,
            outside,
            store,
        }
    }

    fn outside_unchanged(&self) {
        assert_eq!(
            fs::read(self.outside.path().join("secret.txt")).unwrap(),
            OUTSIDE
        );
        assert_eq!(
            fs::read(self.outside.path().join("target.txt")).unwrap(),
            OUTSIDE
        );
        assert_eq!(fs::read_dir(self.outside.path()).unwrap().count(), 2);
    }
}

#[tokio::test]
async fn symlink_files_and_ancestors_cannot_be_read_or_inspected() {
    let fixture = Fixture::new();
    for key in ["project/link/secret.txt", "project/file-link"] {
        let path = Path::from(key);
        assert!(
            fixture.store.get(&path).await.is_err(),
            "get accepted {key}"
        );
        assert!(
            fixture.store.head(&path).await.is_err(),
            "head accepted {key}"
        );
        assert!(
            fixture.store.get_range(&path, 0..4).await.is_err(),
            "range accepted {key}"
        );
        assert!(
            fixture
                .store
                .get_ranges(&path, &[0..4, 4..8])
                .await
                .is_err(),
            "ranges accepted {key}"
        );
        assert!(
            fixture
                .store
                .copy(&path, &Path::from("project/copied.txt"))
                .await
                .is_err(),
            "copy accepted {key}"
        );
    }
    assert!(!fixture.root.path().join("project/copied.txt").exists());
    fixture.outside_unchanged();
}

#[tokio::test]
async fn links_to_siblings_inside_the_store_cannot_escape_a_project_grant() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("project")).unwrap();
    fs::create_dir(root.path().join("private")).unwrap();
    fs::write(root.path().join("private/secret.txt"), OUTSIDE).unwrap();
    symlink("../private", root.path().join("project/relative-link")).unwrap();
    symlink(
        root.path().join("private"),
        root.path().join("project/absolute-link"),
    )
    .unwrap();
    let store = LocalObjectStore::new(root.path().to_path_buf()).unwrap();
    for name in ["relative-link", "absolute-link"] {
        let path = Path::from(format!("project/{name}/secret.txt"));
        assert!(store.get(&path).await.is_err());
        assert!(store.head(&path).await.is_err());
        assert!(
            store
                .put(&path, PutPayload::from_static(b"replacement"))
                .await
                .is_err()
        );
    }
    assert_eq!(
        fs::read(root.path().join("private/secret.txt")).unwrap(),
        OUTSIDE
    );
}

#[tokio::test]
async fn symlink_ancestors_cannot_receive_writes_copies_or_moves() {
    let fixture = Fixture::new();
    let target = Path::from("project/link/target.txt");
    let source = Path::from("project/ordinary.txt");
    assert!(
        fixture
            .store
            .put(&target, PutPayload::from_static(b"replacement"))
            .await
            .is_err()
    );
    assert!(fixture.store.put_multipart(&target).await.is_err());
    assert!(fixture.store.copy(&source, &target).await.is_err());
    assert!(fixture.store.rename(&source, &target).await.is_err());
    assert!(fixture.store.delete(&target).await.is_err());
    assert!(
        fixture
            .store
            .rename(
                &Path::from("project/link/secret.txt"),
                &Path::from("project/stolen.txt")
            )
            .await
            .is_err()
    );
    assert_eq!(
        fs::read(fixture.root.path().join("project/ordinary.txt")).unwrap(),
        b"inside"
    );
    assert!(!fixture.root.path().join("project/stolen.txt").exists());
    fixture.outside_unchanged();
}

#[tokio::test]
async fn final_symlink_mutations_never_change_the_target() {
    let fixture = Fixture::new();
    let source = Path::from("project/ordinary.txt");
    for (name, operation) in [
        ("put-link", 0),
        ("copy-link", 1),
        ("delete-link", 2),
        ("rename-link", 3),
    ] {
        symlink(
            fixture.outside.path().join("target.txt"),
            fixture.root.path().join("project").join(name),
        )
        .unwrap();
        let path = Path::from(format!("project/{name}"));
        match operation {
            0 => {
                let _ = fixture
                    .store
                    .put(&path, PutPayload::from_static(b"replacement"))
                    .await;
            }
            1 => {
                let _ = fixture.store.copy(&source, &path).await;
            }
            2 => {
                let _ = fixture.store.delete(&path).await;
            }
            _ => {
                let _ = fixture.store.rename(&source, &path).await;
            }
        }
        fixture.outside_unchanged();
    }
}

#[tokio::test]
async fn listings_never_expose_objects_behind_symlinks() {
    let fixture = Fixture::new();
    for prefix in [Path::from("project"), Path::from("project/link")] {
        for item in fixture.store.list(Some(&prefix)).collect::<Vec<_>>().await {
            if let Ok(metadata) = item {
                assert!(!metadata.location.as_ref().starts_with("project/link/"));
                assert_ne!(metadata.location.as_ref(), "project/file-link");
            }
        }
        for item in fixture
            .store
            .list_with_offset(Some(&prefix), &Path::from("project/a"))
            .collect::<Vec<_>>()
            .await
        {
            if let Ok(metadata) = item {
                assert!(!metadata.location.as_ref().starts_with("project/link/"));
                assert_ne!(metadata.location.as_ref(), "project/file-link");
            }
        }
        if let Ok(list) = fixture.store.list_with_delimiter(Some(&prefix)).await {
            for metadata in list.objects {
                assert!(!metadata.location.as_ref().starts_with("project/link/"));
                assert_ne!(metadata.location.as_ref(), "project/file-link");
            }
        }
    }
    fixture.outside_unchanged();
}

#[tokio::test]
async fn missing_metadata_through_symlinks_does_not_create_directories() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .store
            .head(&Path::from("project/link/new/nested/missing.txt"))
            .await
            .is_err()
    );
    assert!(!fixture.outside.path().join("new").exists());
    assert!(
        fixture
            .store
            .head(&Path::from("project/new/nested/missing.txt"))
            .await
            .is_err()
    );
    assert!(!fixture.root.path().join("project/new").exists());
}

#[tokio::test]
async fn multipart_commit_keeps_the_open_directory_after_a_symlink_swap() {
    let fixture = Fixture::new();
    let live = fixture.root.path().join("project/live");
    let pinned = fixture.root.path().join("project/pinned");
    fs::create_dir(&live).unwrap();
    let path = Path::from("project/live/target.txt");
    let mut upload = fixture.store.put_multipart(&path).await.unwrap();
    upload
        .put_part(PutPayload::from_static(b"new contents"))
        .await
        .unwrap();
    fs::rename(&live, &pinned).unwrap();
    symlink(fixture.outside.path(), &live).unwrap();
    if upload.complete().await.is_ok() {
        assert_eq!(
            fs::read(pinned.join("target.txt")).unwrap(),
            b"new contents"
        );
    } else {
        let _ = upload.abort().await;
    }
    fixture.outside_unchanged();
}

#[tokio::test]
async fn multipart_abort_does_not_follow_a_replaced_parent() {
    let fixture = Fixture::new();
    let live = fixture.root.path().join("project/live");
    let pinned = fixture.root.path().join("project/pinned");
    fs::create_dir(&live).unwrap();
    let path = Path::from("project/live/target.txt");
    let mut upload = fixture.store.put_multipart(&path).await.unwrap();
    upload
        .put_part(PutPayload::from_static(b"not committed"))
        .await
        .unwrap();
    fs::rename(&live, &pinned).unwrap();
    symlink(fixture.outside.path(), &live).unwrap();
    upload.abort().await.unwrap();
    assert_eq!(fs::read_dir(&pinned).unwrap().count(), 0);
    fixture.outside_unchanged();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_directory_swaps_never_return_outside_bytes() {
    use std::sync::{Arc, Barrier};

    let fixture = Fixture::new();
    let live = fixture.root.path().join("project/live");
    let parked = fixture.root.path().join("project/parked");
    fs::create_dir(&live).unwrap();
    fs::write(live.join("secret.txt"), b"inside").unwrap();
    let outside = fixture.outside.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = barrier.clone();
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        for _ in 0..512 {
            fs::rename(&live, &parked).unwrap();
            symlink(&outside, &live).unwrap();
            std::thread::yield_now();
            fs::remove_file(&live).unwrap();
            fs::rename(&parked, &live).unwrap();
            std::thread::yield_now();
        }
    });
    barrier.wait();
    let mut escaped = false;
    for _ in 0..512 {
        if let Ok(result) = fixture
            .store
            .get(&Path::from("project/live/secret.txt"))
            .await
        {
            if let Ok(bytes) = result.bytes().await {
                escaped |= bytes.as_ref() != b"inside";
            }
        }
    }
    worker.join().unwrap();
    assert!(
        !escaped,
        "a concurrent directory swap exposed outside content"
    );
    fixture.outside_unchanged();
}

#[tokio::test]
async fn ordinary_files_keep_read_write_copy_and_multipart_behavior() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObjectStore::new(root.path().to_path_buf()).unwrap();
    let source = Path::from("project/nested/file.bin");
    let destination = Path::from("project/nested/copy.bin");
    let bytes = Bytes::from_static(&[0, 128, 255, 13, 10]);
    store.put(&source, bytes.clone().into()).await.unwrap();
    assert_eq!(store.head(&source).await.unwrap().size, bytes.len() as u64);
    assert_eq!(
        store.get_ranges(&source, &[0..2, 2..5]).await.unwrap(),
        [bytes.slice(0..2), bytes.slice(2..5)]
    );
    store
        .copy_if_not_exists(&source, &destination)
        .await
        .unwrap();
    assert!(
        store
            .copy_if_not_exists(&source, &destination)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .get(&destination)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        bytes
    );
    let mut upload = store.put_multipart(&destination).await.unwrap();
    upload
        .put_part(PutPayload::from_static(b"first"))
        .await
        .unwrap();
    upload
        .put_part(PutPayload::from_static(b"second"))
        .await
        .unwrap();
    upload.complete().await.unwrap();
    assert_eq!(
        store
            .get(&destination)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        b"firstsecond"[..]
    );
    store.delete(&source).await.unwrap();
    assert!(store.head(&source).await.is_err());
}
