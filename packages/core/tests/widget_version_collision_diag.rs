use flow_like::a2ui::widget::{VersionType, Widget};
use flow_like::app::App;
use flow_like::bit::Metadata;
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store;
use std::sync::Arc;

#[tokio::test]
async fn publishing_from_an_old_version_overwrites_a_published_snapshot() {
    let store = FlowLikeStore::Other(Arc::new(object_store::memory::InMemory::new()));
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::with_default_store(store),
        HTTPClient::new_without_refetch(),
    ));
    let mut app = App::new(
        Some("collision-app".to_string()),
        Metadata::default(),
        Vec::new(),
        state.clone(),
    )
    .await
    .unwrap();

    let mut widget = Widget::new("w", "W", "root");
    widget.version = Some((0, 0, 1));
    widget.name = "v1 content".to_string();
    app.save_widget(&widget).await.unwrap();

    // publish 0.0.2
    let v2 = app
        .create_widget_version("w", VersionType::Patch)
        .await
        .unwrap();
    // edit + publish 0.0.3
    let mut working = app.open_widget("w".to_string(), None).await.unwrap();
    working.name = "v3 content".to_string();
    app.save_widget(&working).await.unwrap();
    let v3 = app
        .create_widget_version("w", VersionType::Patch)
        .await
        .unwrap();
    println!("published {v2:?} and {v3:?}");

    let snap3_before = app.open_widget("w".to_string(), Some(v3)).await.unwrap();
    println!("0.0.3 snapshot name BEFORE: {:?}", snap3_before.name);

    // === what the widget page does when you press "Patch" while ?version=0_0_2 is loaded ===
    // page.tsx: updateWidget(appId, {...widget}) with `widget` = the LOADED SNAPSHOT
    let snapshot2 = app.open_widget("w".to_string(), Some(v2)).await.unwrap();
    app.save_widget(&snapshot2).await.unwrap(); // working copy regresses to 0.0.2
    let regressed = app.open_widget("w".to_string(), None).await.unwrap();
    println!(
        "working copy after loading v0.0.2 and saving: version={:?} name={:?}",
        regressed.version, regressed.name
    );

    let again = app
        .create_widget_version("w", VersionType::Patch)
        .await
        .unwrap();
    println!("second publish returned {again:?} (already existed: {})", again == v3);

    let snap3_after = app.open_widget("w".to_string(), Some(v3)).await.unwrap();
    println!("0.0.3 snapshot name AFTER:  {:?}", snap3_after.name);
    println!("versions listed: {:?}", app.get_widget_versions("w").await.unwrap());

    assert_eq!(
        snap3_before.name, "v3 content",
        "sanity: the 0.0.3 snapshot captured the v3 edit"
    );
    assert_eq!(
        snap3_after.name, "v3 content",
        "PUBLISHED SNAPSHOT 0.0.3 WAS SILENTLY REWRITTEN"
    );
}
