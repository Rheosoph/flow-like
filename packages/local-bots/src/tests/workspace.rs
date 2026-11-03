use super::*;
use crate::{WorkspaceBot, WorkspaceHost, codes};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Barrier;

type Hook = Box<dyn FnOnce() + Send>;

/// Apps and memories in memory, with every call it was asked.
#[derive(Default)]
struct Host {
    /// App id → the name of the bot it was made for.
    apps: Mutex<BTreeMap<String, String>>,
    memories: Mutex<BTreeSet<String>>,
    calls: Mutex<Vec<String>>,
    made: AtomicUsize,
    /// App and bot ids whose deletion fails.
    broken: Mutex<BTreeSet<String>>,
    /// Makers of a new app wait here once they made it.
    race: Option<Barrier>,
    /// Runs once, right after a new app was made.
    after_create: Mutex<Option<Hook>>,
    /// Runs once, right after a bot's memory was deleted.
    after_forget: Mutex<Option<Hook>>,
}

impl Host {
    fn racing(makers: usize) -> Self {
        Self {
            race: Some(Barrier::new(makers)),
            ..Self::default()
        }
    }

    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn apps(&self) -> Vec<String> {
        self.apps.lock().unwrap().keys().cloned().collect()
    }

    fn made_for(&self, app_id: &str) -> Option<String> {
        self.apps.lock().unwrap().get(app_id).cloned()
    }

    fn remember(&self, bot_id: &str) {
        self.memories.lock().unwrap().insert(bot_id.to_string());
    }

    fn remembers(&self, bot_id: &str) -> bool {
        self.memories.lock().unwrap().contains(bot_id)
    }

    fn break_deletion(&self, id: &str) {
        self.broken.lock().unwrap().insert(id.to_string());
    }

    fn repair(&self) {
        self.broken.lock().unwrap().clear();
    }

    fn refuses(&self, id: &str) -> Result<(), String> {
        if self.broken.lock().unwrap().contains(id) {
            return Err(format!("{id} is in use"));
        }
        Ok(())
    }

    fn run(hook: &Mutex<Option<Hook>>) {
        let hook = hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }
}

impl WorkspaceHost for Host {
    async fn create_app(
        &self,
        bot: WorkspaceBot<'_>,
        app_id: Option<&str>,
    ) -> Result<String, String> {
        let app_id = match app_id {
            Some(app_id) => app_id.to_string(),
            None => format!("app-{}", self.made.fetch_add(1, Ordering::SeqCst) + 1),
        };
        self.note(format!("create {app_id} for {}", bot.id));
        self.apps
            .lock()
            .unwrap()
            .insert(app_id.clone(), bot.name.to_string());
        if let Some(race) = &self.race {
            race.wait().await;
        }
        Self::run(&self.after_create);
        Ok(app_id)
    }

    async fn app_exists(&self, app_id: &str) -> Result<bool, String> {
        Ok(self.apps.lock().unwrap().contains_key(app_id))
    }

    async fn delete_app(&self, app_id: &str) -> Result<(), String> {
        self.note(format!("delete {app_id}"));
        self.refuses(app_id)?;
        self.apps.lock().unwrap().remove(app_id);
        Ok(())
    }

    async fn delete_memory(&self, bot_id: &str) -> Result<(), String> {
        self.note(format!("forget {bot_id}"));
        self.refuses(bot_id)?;
        self.memories.lock().unwrap().remove(bot_id);
        Self::run(&self.after_forget);
        Ok(())
    }
}

fn stored_workspace(f: &Fixture, bot_id: &str) -> Option<String> {
    f.store.get_bot(bot_id).unwrap().unwrap().workspace_app_id
}

#[tokio::test]
async fn a_bot_gets_its_workspace_on_first_use_and_the_same_one_after() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    f.changes.take();

    let app_id = f.store.workspace(&host, &scout.id).await.unwrap();
    assert_eq!(app_id, "app-1");
    assert_eq!(host.made_for(&app_id).as_deref(), Some("Scout"));
    assert_eq!(f.store.workspace(&host, &scout.id).await.unwrap(), app_id);
    assert_eq!(host.calls(), [format!("create app-1 for {}", scout.id)]);
    assert_eq!(stored_workspace(&f, &scout.id).as_deref(), Some("app-1"));

    let stored = f.store.get_bot(&scout.id).unwrap().unwrap();
    assert_eq!(
        stored.updated_at, scout.updated_at,
        "a workspace isn't on the wire, so a move's conditional forget still holds"
    );
    assert!(f.changes.take().is_empty());
    assert!(
        serde_json::to_value(&stored)
            .unwrap()
            .get("workspace_app_id")
            .is_none()
    );
}

#[tokio::test]
async fn first_uses_that_race_keep_the_first_app_and_delete_the_others() {
    let f = fixture();
    let scout = f.bot("Scout");
    let other_window = Arc::new(f.open_again(TestKey::default()).unwrap());
    let host = Host::racing(2);

    let (first, second) = tokio::join!(
        f.store.workspace(&host, &scout.id),
        other_window.workspace(&host, &scout.id),
    );

    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first, second, "both get the app that was recorded first");
    assert_eq!(stored_workspace(&f, &scout.id), Some(first.clone()));
    assert_eq!(
        host.apps(),
        std::slice::from_ref(&first),
        "the other app is deleted"
    );
    let lost = if first == "app-1" { "app-2" } else { "app-1" };
    assert!(host.calls().contains(&format!("delete {lost}")));
    assert!(!host.calls().contains(&format!("delete {first}")));
}

#[tokio::test]
async fn an_app_made_for_a_bot_deleted_meanwhile_is_deleted_again() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    let (store, bot_id) = (f.store.clone(), scout.id.clone());
    *host.after_create.lock().unwrap() = Some(Box::new(move || {
        store.delete_bot(&bot_id).unwrap();
    }));

    let error = f.store.workspace(&host, &scout.id).await.unwrap_err();

    assert_eq!(error.code(), codes::NOT_FOUND);
    assert!(host.apps().is_empty());
    assert_eq!(
        host.calls(),
        [
            format!("create app-1 for {}", scout.id),
            "delete app-1".into()
        ]
    );
}

#[tokio::test]
async fn flux_has_no_workspace_but_assistant_and_flux_copies_do() {
    let f = fixture();
    let host = Host::default();
    f.store.ensure_built_ins().unwrap();
    let bots = f.store.list_bots().unwrap();
    let (general, flux) = (&bots[0], &bots[1]);

    let error = f.store.workspace(&host, &flux.id).await.unwrap_err();
    assert_eq!(error.code(), codes::BOT_WORKSPACE_REFUSED);
    assert_eq!(error.message(), "Flux can't have a workspace yet");
    assert!(host.calls().is_empty());
    assert_eq!(stored_workspace(&f, &flux.id), None);

    let assistant = f.store.workspace(&host, &general.id).await.unwrap();
    assert_eq!(host.made_for(&assistant).as_deref(), Some("Assistant"));
    let copy = f.store.clone_bot(&flux.id).unwrap();
    let copied = f.store.workspace(&host, &copy.id).await.unwrap();
    assert_eq!(host.made_for(&copied).as_deref(), Some("Flux copy"));
    assert_ne!(assistant, copied);
}

#[tokio::test]
async fn a_duplicate_gets_a_workspace_of_its_own() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    let original = f.store.workspace(&host, &scout.id).await.unwrap();

    let copy = f.store.clone_bot(&scout.id).unwrap();
    assert_eq!(copy.workspace_app_id, None);
    let copied = f.store.workspace(&host, &copy.id).await.unwrap();

    assert_ne!(copied, original);
    assert_eq!(stored_workspace(&f, &scout.id), Some(original));
}

#[tokio::test]
async fn a_workspace_whose_app_went_missing_comes_back_under_its_id() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    let app_id = f.store.workspace(&host, &scout.id).await.unwrap();
    host.apps.lock().unwrap().clear();

    assert_eq!(f.store.workspace(&host, &scout.id).await.unwrap(), app_id);

    assert_eq!(host.apps(), std::slice::from_ref(&app_id));
    assert_eq!(
        host.calls(),
        [
            format!("create {app_id} for {}", scout.id),
            format!("create {app_id} for {}", scout.id)
        ]
    );
}

#[tokio::test]
async fn an_unknown_bot_has_no_workspace_and_nothing_to_delete() {
    let f = fixture();
    let host = Host::default();
    assert_eq!(
        code(f.store.workspace(&host, "bot_missing").await),
        codes::NOT_FOUND
    );
    assert_eq!(
        code(
            f.store
                .delete_bot_with_workspace(&host, "bot_missing")
                .await
        ),
        codes::NOT_FOUND
    );
    assert!(host.calls().is_empty());
}

#[tokio::test]
async fn deleting_a_bot_deletes_its_workspace_and_memory() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    let keeper = f.bot("Keeper");
    let app_id = f.store.workspace(&host, &scout.id).await.unwrap();
    let kept = f.store.workspace(&host, &keeper.id).await.unwrap();
    host.remember(&scout.id);
    host.remember(&keeper.id);
    f.changes.take();

    f.store
        .delete_bot_with_workspace(&host, &scout.id)
        .await
        .unwrap();

    assert_eq!(f.store.get_bot(&scout.id).unwrap(), None);
    assert_eq!(host.apps(), [kept]);
    assert!(!host.remembers(&scout.id));
    assert!(host.remembers(&keeper.id));
    assert!(host.calls().contains(&format!("delete {app_id}")));
    assert!(f.changes.take().contains(&Change::Bots(Some(scout.id))));
}

#[tokio::test]
async fn a_failed_delete_keeps_the_bot_and_deleting_again_finishes() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    let app_id = f.store.workspace(&host, &scout.id).await.unwrap();
    host.remember(&scout.id);

    host.break_deletion(&app_id);
    let error = f
        .store
        .delete_bot_with_workspace(&host, &scout.id)
        .await
        .unwrap_err();
    assert_eq!(error.code(), codes::BOT_WORKSPACE_FAILED);
    assert!(error.message().contains(&app_id), "{error}");
    assert!(error.message().contains("is in use"), "{error}");
    assert_eq!(stored_workspace(&f, &scout.id), Some(app_id.clone()));
    assert!(host.remembers(&scout.id), "memory goes after the app");

    host.repair();
    host.break_deletion(&scout.id);
    let error = f
        .store
        .delete_bot_with_workspace(&host, &scout.id)
        .await
        .unwrap_err();
    assert!(error.message().contains("memory"), "{error}");
    assert!(f.store.get_bot(&scout.id).unwrap().is_some());
    assert!(host.apps().is_empty());

    host.repair();
    f.store
        .delete_bot_with_workspace(&host, &scout.id)
        .await
        .unwrap();
    assert_eq!(f.store.get_bot(&scout.id).unwrap(), None);
    assert!(!host.remembers(&scout.id));
}

#[tokio::test]
async fn a_workspace_made_while_its_bot_is_deleted_goes_too() {
    let f = fixture();
    let host = Host::default();
    let scout = f.bot("Scout");
    host.apps
        .lock()
        .unwrap()
        .insert("app-late".into(), "Scout".into());
    let (path, bot_id) = (f.path.clone(), scout.id.clone());
    *host.after_forget.lock().unwrap() = Some(Box::new(move || {
        rusqlite::Connection::open(path)
            .unwrap()
            .execute(
                "UPDATE bots SET workspace_app_id = 'app-late' WHERE id = ?1",
                [bot_id],
            )
            .unwrap();
    }));

    f.store
        .delete_bot_with_workspace(&host, &scout.id)
        .await
        .unwrap();

    assert_eq!(f.store.get_bot(&scout.id).unwrap(), None);
    assert!(host.apps().is_empty());
}

#[tokio::test]
async fn built_ins_keep_their_workspace_and_memory() {
    let f = fixture();
    let host = Host::default();
    f.store.ensure_built_ins().unwrap();
    let general = f.store.list_bots().unwrap().remove(0);
    let app_id = f.store.workspace(&host, &general.id).await.unwrap();
    host.remember(&general.id);

    let error = f
        .store
        .delete_bot_with_workspace(&host, &general.id)
        .await
        .unwrap_err();

    assert_eq!(error.code(), codes::BOT_SYSTEM_UNDELETABLE);
    assert_eq!(host.apps(), [app_id]);
    assert!(host.remembers(&general.id));
    assert_eq!(host.calls().len(), 1, "{:?}", host.calls());
}
