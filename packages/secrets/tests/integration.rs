use flow_like_secrets::ExposeSecret;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, FileProviderConfig, ProviderConfig, SecretProviderKind,
    SecretRef, SecretStore, SecretStoreConfig,
};
use std::sync::Arc;

fn must_ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

fn reveal(secret: flow_like_secrets::SecretValue) -> String {
    must_ok(secret.as_text(), "secret must be text")
        .expose_secret()
        .to_string()
}

#[tokio::test]
async fn reads_secret_from_file_provider() {
    let temp_dir = must_ok(tempfile::tempdir(), "must create temp dir");
    let secret_path = temp_dir.path().join("api-key");
    must_ok(
        tokio::fs::write(&secret_path, b"super-secret\n").await,
        "must write secret",
    );

    let config =
        SecretStoreConfig::default().with_provider(ProviderConfig::File(FileProviderConfig {
            root_path: temp_dir.path().to_path_buf(),
            trim_trailing_newline: true,
        }));

    let store = must_ok(SecretStore::new(config), "must create store");
    let value = reveal(must_ok(
        store.get_secret(&SecretRef::new("api-key")).await,
        "must read secret",
    ));

    assert_eq!(value, "super-secret");
}

#[tokio::test]
async fn caches_secret_after_first_read() {
    let temp_dir = must_ok(tempfile::tempdir(), "must create temp dir");
    let secret_path = temp_dir.path().join("token");
    must_ok(
        tokio::fs::write(&secret_path, b"first").await,
        "must write secret",
    );

    let config = SecretStoreConfig {
        global_prefix: None,
        cache_ttl: std::time::Duration::from_secs(30),
        negative_cache_ttl: std::time::Duration::from_secs(1),
        max_cache_entries: 128,
        providers: vec![ProviderConfig::File(FileProviderConfig {
            root_path: temp_dir.path().to_path_buf(),
            trim_trailing_newline: true,
        })],
    };

    let store = must_ok(SecretStore::new(config), "must create store");
    let reference = SecretRef::with_provider(SecretProviderKind::File, "token");

    let first = reveal(must_ok(
        store.get_secret(&reference).await,
        "must read first value",
    ));
    assert_eq!(first, "first");

    must_ok(
        tokio::fs::remove_file(&secret_path).await,
        "must remove secret source",
    );

    let second = reveal(must_ok(
        store.get_secret(&reference).await,
        "must read cached value",
    ));
    assert_eq!(second, "first");
}

#[tokio::test]
async fn invalidate_forces_reload() {
    let temp_dir = must_ok(tempfile::tempdir(), "must create temp dir");
    let secret_path = temp_dir.path().join("db-password");
    must_ok(
        tokio::fs::write(&secret_path, b"v1").await,
        "must write secret",
    );

    let config =
        SecretStoreConfig::default().with_provider(ProviderConfig::File(FileProviderConfig {
            root_path: temp_dir.path().to_path_buf(),
            trim_trailing_newline: true,
        }));

    let store = must_ok(SecretStore::new(config), "must create store");
    let reference = SecretRef::with_provider(SecretProviderKind::File, "db-password");

    let first = reveal(must_ok(
        store.get_secret(&reference).await,
        "must read first value",
    ));
    assert_eq!(first, "v1");

    must_ok(
        tokio::fs::write(&secret_path, b"v2").await,
        "must rotate secret",
    );
    store.invalidate(&reference).await;

    let second = reveal(must_ok(
        store.get_secret(&reference).await,
        "must read rotated value",
    ));
    assert_eq!(second, "v2");
}

#[tokio::test]
async fn lazy_provider_initialization_does_not_fail_unrelated_lookup() {
    let temp_dir = must_ok(tempfile::tempdir(), "must create temp dir");
    let secret_path = temp_dir.path().join("service-token");
    must_ok(
        tokio::fs::write(&secret_path, b"ok").await,
        "must write secret",
    );

    let config = SecretStoreConfig {
        global_prefix: None,
        cache_ttl: std::time::Duration::from_secs(60),
        negative_cache_ttl: std::time::Duration::from_secs(5),
        max_cache_entries: 128,
        providers: vec![
            ProviderConfig::AwsParameterStore(AwsParameterStoreProviderConfig::default()),
            ProviderConfig::File(FileProviderConfig {
                root_path: temp_dir.path().to_path_buf(),
                trim_trailing_newline: true,
            }),
        ],
    };

    let store = Arc::new(must_ok(SecretStore::new(config), "must create store"));

    let value = reveal(must_ok(
        store
            .get_secret(&SecretRef::with_provider(
                SecretProviderKind::File,
                "service-token",
            ))
            .await,
        "file provider lookup must succeed",
    ));

    assert_eq!(value, "ok");
}

#[tokio::test]
async fn fallback_order_uses_first_matching_provider() {
    let temp_dir = must_ok(tempfile::tempdir(), "must create temp dir");
    let secret_path = temp_dir.path().join("shared-key");
    must_ok(
        tokio::fs::write(&secret_path, b"file-value").await,
        "must write secret",
    );

    let config = SecretStoreConfig {
        global_prefix: None,
        cache_ttl: std::time::Duration::from_secs(60),
        negative_cache_ttl: std::time::Duration::from_secs(5),
        max_cache_entries: 128,
        providers: vec![
            ProviderConfig::docker_compose(Some("MISSING".to_string())),
            ProviderConfig::File(FileProviderConfig {
                root_path: temp_dir.path().to_path_buf(),
                trim_trailing_newline: true,
            }),
        ],
    };

    let store = must_ok(SecretStore::new(config), "must create store");
    let value = reveal(must_ok(
        store.get_secret(&SecretRef::new("shared-key")).await,
        "fallback should resolve",
    ));

    assert_eq!(value, "file-value");
}

#[tokio::test]
async fn env_override_wins_before_explicit_provider_lookup() {
    let expected = must_ok(std::env::var("PATH"), "PATH must be set for test");
    let temp_dir = must_ok(tempfile::tempdir(), "must create temp dir");
    let secret_path = temp_dir.path().join("PATH");
    must_ok(
        tokio::fs::write(&secret_path, b"file-value").await,
        "must write secret",
    );

    let config =
        SecretStoreConfig::default().with_provider(ProviderConfig::File(FileProviderConfig {
            root_path: temp_dir.path().to_path_buf(),
            trim_trailing_newline: true,
        }));

    let store = must_ok(SecretStore::new(config), "must create store");
    let value = reveal(must_ok(
        store
            .get_secret(&SecretRef::with_provider(SecretProviderKind::File, "PATH"))
            .await,
        "env override should resolve before file provider",
    ));

    assert_eq!(value, expected);
}
