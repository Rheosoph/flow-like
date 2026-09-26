//! Copy-on-write ownership of one open board.
//!
//! Readers take an `Arc<Board>` snapshot without waiting on anything. A published snapshot is never
//! mutated, so whatever is built from it — a run template, a sync diff, an IPC response — sees one
//! consistent revision, and a slow writer never delays a reader.
//!
//! Writers are serialised among themselves and edit a private draft, copied from the published board
//! on the first mutable access. Dropping the writer publishes the draft; [`BoardWriter::discard`]
//! drops it instead, which is the whole rollback. A reader therefore never observes a half-applied
//! or rolled-back edit, and a writer that only reads costs no copy and publishes nothing.

use super::Board;
use flow_like_types::tokio::sync::{Mutex, MutexGuard};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard};

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

pub struct BoardCell {
    /// The published board and its revision, unique across every cell in the process. Held only
    /// for a pointer copy or swap, never across an await.
    published: RwLock<(Arc<Board>, u64)>,
    writer: Mutex<()>,
}

impl BoardCell {
    pub fn new(board: Board) -> Self {
        Self {
            published: RwLock::new((Arc::new(board), next_revision())),
            writer: Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> Arc<Board> {
        self.published().0.clone()
    }

    /// The published board together with its revision. Derived artefacts keyed on the revision
    /// (compiled templates, run requirements) stay valid for exactly as long as this snapshot.
    pub fn snapshot_with_revision(&self) -> (Arc<Board>, u64) {
        self.published().clone()
    }

    /// Waits for earlier writers only; readers are never blocked by the returned writer.
    pub async fn write(&self) -> BoardWriter<'_> {
        let permit = self.writer.lock().await;
        BoardWriter {
            cell: self,
            _permit: permit,
            base: self.snapshot(),
            draft: None,
        }
    }

    fn published(&self) -> RwLockReadGuard<'_, (Arc<Board>, u64)> {
        self.published
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish(&self, board: Board) {
        let mut published = self
            .published
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *published = (Arc::new(board), next_revision());
    }
}

pub struct BoardWriter<'a> {
    cell: &'a BoardCell,
    _permit: MutexGuard<'a, ()>,
    base: Arc<Board>,
    draft: Option<Board>,
}

impl BoardWriter<'_> {
    /// The board as published when this writer started, unaffected by the draft.
    pub fn original(&self) -> &Arc<Board> {
        &self.base
    }

    /// Drops every change made through this writer; it keeps the writer permit.
    pub fn discard(&mut self) {
        self.draft = None;
    }
}

impl Deref for BoardWriter<'_> {
    type Target = Board;

    fn deref(&self) -> &Board {
        self.draft.as_ref().unwrap_or(&self.base)
    }
}

impl DerefMut for BoardWriter<'_> {
    fn deref_mut(&mut self) -> &mut Board {
        self.draft.get_or_insert_with(|| (*self.base).clone())
    }
}

impl Drop for BoardWriter<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        if let Some(draft) = self.draft.take() {
            self.cell.publish(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::path::Path;
    use flow_like_types::tokio;

    fn board(name: &str) -> Board {
        let mut board = Board::new_detached(None, Path::from("boards"));
        board.name = name.to_string();
        board
    }

    #[tokio::test]
    async fn readers_keep_their_snapshot_while_a_writer_publishes() {
        let cell = BoardCell::new(board("before"));
        let (held, held_revision) = cell.snapshot_with_revision();

        {
            let mut writer = cell.write().await;
            writer.name = "after".to_string();
            assert_eq!(cell.snapshot().name, "before");
        }

        assert_eq!(held.name, "before");
        let (current, revision) = cell.snapshot_with_revision();
        assert_eq!(current.name, "after");
        assert!(revision > held_revision);
    }

    #[tokio::test]
    async fn a_writer_that_only_reads_publishes_nothing() {
        let cell = BoardCell::new(board("same"));
        let (before, revision) = cell.snapshot_with_revision();

        {
            let writer = cell.write().await;
            assert_eq!(writer.name, "same");
        }

        let (after, after_revision) = cell.snapshot_with_revision();
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(after_revision, revision);
    }

    #[tokio::test]
    async fn discard_rolls_the_draft_back() {
        let cell = BoardCell::new(board("kept"));
        let (_, revision) = cell.snapshot_with_revision();

        {
            let mut writer = cell.write().await;
            writer.name = "dropped".to_string();
            writer.discard();
            assert_eq!(writer.name, "kept");
            assert_eq!(writer.original().name, "kept");
        }

        let (current, current_revision) = cell.snapshot_with_revision();
        assert_eq!(current.name, "kept");
        assert_eq!(current_revision, revision);
    }

    #[tokio::test]
    async fn writers_queue_behind_each_other() {
        let cell = Arc::new(BoardCell::new(board("0")));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let cell = cell.clone();
            tasks.push(tokio::spawn(async move {
                let mut writer = cell.write().await;
                let next = writer.name.parse::<u32>().unwrap() + 1;
                tokio::task::yield_now().await;
                writer.name = next.to_string();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(cell.snapshot().name, "16");
    }
}
