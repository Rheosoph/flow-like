#![cfg(unix)]

use anyhow::{Context, Result, ensure};
use flow_like_standalone::{
    config::{EventBinding, PlacementConfig, ProjectSource, RestartPolicy},
    state::{DesiredState, ObservedState, StateStore},
    supervisor,
};
use std::{
    collections::BTreeMap, future::Future, os::unix::fs::PermissionsExt, path::PathBuf,
    time::Duration,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Fixture {
    directory: TempDir,
    program: PathBuf,
}

impl Fixture {
    fn new(script: &str, desired: DesiredState) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let state_dir = supervisor::prepare_state_dir(directory.path())?;
        let program = state_dir.join("fake-workload");
        std::fs::write(&program, script)?;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;
        let config = PlacementConfig {
            id: "service".into(),
            project_id: "project".into(),
            deployment_id: "deployment".into(),
            revision: "revision-1".into(),
            source: ProjectSource::Offline,
            online_metadata_sha256: None,
            project_path: state_dir,
            events: vec![EventBinding {
                event_id: "daemon".into(),
                event_version: [1, 0, 0],
                board_version: [1, 0, 0],
            }],
            variables: BTreeMap::new(),
            secret_overrides: BTreeMap::new(),
            resource_grant: None,
            max_replicas: 1,
            tls_certificate_id: None,
            hosting: None,
            artifact_pins: vec![],
            bit_pins: vec![],
            package_pins: vec![],
            offline_writes: None,
            resources: None,
            restart: RestartPolicy {
                initial_backoff_secs: 1,
                max_backoff_secs: 1,
                max_restarts: 1,
            },
        };
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("service", &serde_json::to_value(config)?, desired)?;
        Ok(Self { directory, program })
    }

    fn store(&self) -> Result<StateStore> {
        StateStore::open(&self.directory.path().join("management.sqlite"))
    }

    fn starts(&self) -> Result<usize> {
        match std::fs::read_to_string(self.directory.path().join("starts")) {
            Ok(starts) => Ok(starts.lines().count()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error.into()),
        }
    }

    async fn supervise_until(&self, observe: impl Future<Output = Result<()>>) -> Result<()> {
        let cancel = CancellationToken::new();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let observe = async {
            let result = observe.await;
            cancel.cancel();
            result
        };
        let (supervised, observed) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(
                supervisor::run(self.directory.path(), &self.program, cancel.clone()),
                observe
            )
        })
        .await
        .context("supervisor integration test exceeded its deadline")?;
        observed?;
        supervised
    }
}

async fn wait_for(label: &'static str, mut predicate: impl FnMut() -> Result<bool>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if predicate()? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {label}"))?
}

// The executable replaces itself with sleep, so even a failed test has no orphan descendants.
const PERSISTENT_WORKLOAD: &str =
    "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$2/starts\"\nexec /bin/sleep 30\n";

#[tokio::test(flavor = "current_thread")]
async fn stopped_intent_survives_supervisor_restarts_without_spawning() -> Result<()> {
    let fixture = Fixture::new(PERSISTENT_WORKLOAD, DesiredState::Running)?;
    let device_id = fixture.store()?.device_id().to_owned();
    let mut store = fixture.store()?;
    fixture
        .supervise_until(async {
            wait_for("workload before stop", || {
                Ok(fixture.starts()? == 1
                    && store
                        .get_placement("service")?
                        .unwrap()
                        .process_id
                        .is_some())
            })
            .await?;
            store.set_desired_state("service", DesiredState::Stopped)?;
            wait_for("explicit workload stop", || {
                let record = store.get_placement("service")?.unwrap();
                Ok(record.observed_state == ObservedState::Stopped && record.process_id.is_none())
            })
            .await?;
            Ok(())
        })
        .await?;

    {
        let store = fixture.store()?;
        fixture
            .supervise_until(async {
                wait_for("stopped reconciliation", || {
                    Ok(store.get_placement("service")?.unwrap().observed_state
                        == ObservedState::Stopped)
                })
                .await?;
                tokio::time::sleep(Duration::from_millis(1100)).await;
                ensure!(
                    fixture.starts()? == 1,
                    "stopped service was unexpectedly restarted"
                );
                Ok(())
            })
            .await?;
        assert_eq!(store.device_id(), device_id);
        let record = store.get_placement("service")?.unwrap();
        assert_eq!(record.desired_state, DesiredState::Stopped);
        assert_eq!(record.observed_state, ObservedState::Stopped);
        assert_eq!(record.process_id, None);
        assert!(!supervisor::agent_is_running(fixture.directory.path())?);
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_reaps_live_workload_and_preserves_running_intent() -> Result<()> {
    let fixture = Fixture::new(PERSISTENT_WORKLOAD, DesiredState::Running)?;
    let store = fixture.store()?;
    let mut process_id = None;
    fixture
        .supervise_until(async {
            wait_for("live workload", || {
                let record = store.get_placement("service")?.unwrap();
                Ok(fixture.starts()? == 1 && record.process_id.is_some())
            })
            .await?;
            let record = store.get_placement("service")?.unwrap();
            ensure!(record.observed_state == ObservedState::Starting);
            ensure!(
                record.applied_revision.is_none(),
                "spawn claimed readiness without acknowledgement"
            );
            process_id = record.process_id;
            ensure!(supervisor::agent_is_running(fixture.directory.path())?);
            Ok(())
        })
        .await?;

    let record = store.get_placement("service")?.unwrap();
    assert_eq!(record.desired_state, DesiredState::Running);
    assert_eq!(record.observed_state, ObservedState::Stopped);
    assert_eq!(record.process_id, None);
    assert!(!supervisor::agent_is_running(fixture.directory.path())?);
    // Signal zero only checks the child PID from this test; it cannot terminate a process.
    assert_eq!(
        unsafe { libc::kill(process_id.unwrap() as libc::pid_t, 0) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn crash_loop_stops_at_limit_and_explicit_start_recovers() -> Result<()> {
    let fixture = Fixture::new(
        "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$2/starts\"\nif [ -f \"$2/recovered\" ]; then exec /bin/sleep 30; fi\nexit 42\n",
        DesiredState::Running,
    )?;
    let mut store = fixture.store()?;
    fixture
        .supervise_until(async {
            wait_for("crash backoff", || {
                Ok(store.get_placement("service")?.unwrap().observed_state
                    == ObservedState::Backoff)
            })
            .await?;
            ensure!(fixture.starts()? == 1);

            wait_for("crash-loop limit", || {
                Ok(
                    store.get_placement("service")?.unwrap().observed_state
                        == ObservedState::Failed,
                )
            })
            .await?;
            ensure!(fixture.starts()? == 2);
            tokio::time::sleep(Duration::from_millis(1100)).await;
            ensure!(
                fixture.starts()? == 2,
                "supervisor exceeded its crash-loop limit"
            );

            std::fs::write(fixture.directory.path().join("recovered"), [])?;
            store.set_desired_state("service", DesiredState::Running)?;
            wait_for("explicit-start recovery", || {
                let record = store.get_placement("service")?.unwrap();
                Ok(fixture.starts()? == 3
                    && record.observed_state == ObservedState::Starting
                    && record.process_id.is_some())
            })
            .await?;
            tokio::time::sleep(Duration::from_millis(1100)).await;
            ensure!(fixture.starts()? == 3, "healthy workload was restarted");
            Ok(())
        })
        .await?;
    assert_eq!(
        store.get_placement("service")?.unwrap().observed_state,
        ObservedState::Stopped
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_placement_does_not_interrupt_its_healthy_peer() -> Result<()> {
    let fixture = Fixture::new(PERSISTENT_WORKLOAD, DesiredState::Running)?;
    let mut store = fixture.store()?;
    fixture
        .supervise_until(async {
            wait_for("healthy peer", || {
                Ok(fixture.starts()? == 1
                    && store
                        .get_placement("service")?
                        .unwrap()
                        .process_id
                        .is_some())
            })
            .await?;
            let healthy_pid = store.get_placement("service")?.unwrap().process_id;
            store.upsert_placement(
                "broken",
                &serde_json::json!({"id": "broken", "events": "invalid event list"}),
                DesiredState::Running,
            )?;
            wait_for("malformed placement failure", || {
                Ok(store.get_placement("broken")?.unwrap().observed_state == ObservedState::Failed)
            })
            .await?;
            let failed = store.get_placement("broken")?.unwrap();
            ensure!(failed.process_id.is_none());
            ensure!(
                failed.last_error.as_deref() == Some("Invalid persisted placement configuration")
            );
            tokio::time::sleep(Duration::from_millis(1100)).await;
            ensure!(supervisor::agent_is_running(fixture.directory.path())?);
            ensure!(fixture.starts()? == 1, "healthy peer was restarted");
            ensure!(store.get_placement("service")?.unwrap().process_id == healthy_pid);

            store.set_desired_state("broken", DesiredState::Stopped)?;
            wait_for("malformed placement stop", || {
                Ok(
                    store.get_placement("broken")?.unwrap().observed_state
                        == ObservedState::Stopped,
                )
            })
            .await?;
            store.remove_placement("broken")?;
            ensure!(store.get_placement("service")?.unwrap().process_id == healthy_pid);
            Ok(())
        })
        .await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn scale_preserves_existing_process_and_reaps_only_excess_slots() -> Result<()> {
    let fixture = Fixture::new(PERSISTENT_WORKLOAD, DesiredState::Running)?;
    let mut store = fixture.store()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let mut config = store.get_placement("service")?.unwrap().config;
    config["max_replicas"] = serde_json::json!(3);
    config["hosting"] = serde_json::json!({"host":"127.0.0.1","port":port,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"});
    let record = store.upsert_placement("service", &config, DesiredState::Running)?;
    fixture
        .supervise_until(async {
            wait_for("first replica", || {
                Ok(store.get_placement("service")?.unwrap().running_replicas == 1)
            })
            .await?;
            let first = store.get_placement("service")?.unwrap().replicas[0].process_id;
            store.set_replica_count("service", record.config_revision, 3)?;
            wait_for("three replicas", || {
                Ok(
                    store.get_placement("service")?.unwrap().running_replicas == 3
                        && fixture.starts()? == 3,
                )
            })
            .await?;
            assert_eq!(
                store.get_placement("service")?.unwrap().replicas[0].process_id,
                first
            );
            assert_eq!(fixture.starts()?, 3);
            store.set_replica_count("service", record.config_revision, 1)?;
            wait_for("one retained replica", || {
                Ok(store.get_placement("service")?.unwrap().running_replicas == 1)
            })
            .await?;
            assert_eq!(
                store.get_placement("service")?.unwrap().replicas[0].process_id,
                first
            );
            assert_eq!(fixture.starts()?, 3);
            Ok(())
        })
        .await?;
    assert_eq!(store.get_placement("service")?.unwrap().running_replicas, 0);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn updated_replica_cohort_waits_for_every_previous_worker_to_drain() -> Result<()> {
    const WORKLOAD: &str = r#"#!/bin/sh
task_root="$2"
task_slot="$6"
task_revision="$8"
read task_old_revision < "$task_root/old-revision"
if [ "$task_revision" = "$task_old_revision" ]; then
  printf '%s\n' "$$" > "$task_root/old-$task_slot.pid"
  if [ "$task_slot" = 1 ]; then
    trap 'trap "" TERM; printf draining > "$task_root/slow-draining"; while [ ! -f "$task_root/release-drain" ]; do /bin/sleep 0.05; done; exit 0' TERM
  else
    trap 'printf exited > "$task_root/fast-exited"; exit 0' TERM
  fi
  printf '%s:%s\n' "$task_revision" "$task_slot" >> "$task_root/starts"
  while :; do /bin/sleep 0.05; done
fi
for task_file in "$task_root"/old-*.pid; do
  read task_old_pid < "$task_file"
  if kill -0 "$task_old_pid" 2>/dev/null; then
    printf '%s:%s\n' "$task_revision" "$task_slot" >> "$task_root/overlap"
  fi
done
printf '%s:%s\n' "$task_revision" "$task_slot" >> "$task_root/starts"
exec /bin/sleep 30
"#;

    struct ReleaseDrain(PathBuf);
    impl Drop for ReleaseDrain {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, []);
        }
    }

    let fixture = Fixture::new(WORKLOAD, DesiredState::Running)?;
    let mut store = fixture.store()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let mut config = store.get_placement("service")?.unwrap().config;
    config["max_replicas"] = serde_json::json!(2);
    config["hosting"] = serde_json::json!({"host":"127.0.0.1","port":port,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"});
    let previous = store.upsert_placement("service", &config, DesiredState::Running)?;
    store.set_replica_count("service", previous.config_revision, 2)?;
    std::fs::write(
        fixture.directory.path().join("old-revision"),
        format!("{}\n", previous.config_revision),
    )?;
    let release = fixture.directory.path().join("release-drain");
    fixture
        .supervise_until(async {
            let _release_on_failure = ReleaseDrain(release.clone());
            wait_for("both previous workers", || {
                Ok(fixture.starts()? == 2
                    && store.get_placement("service")?.unwrap().running_replicas == 2)
            })
            .await?;
            let old_pids: Vec<_> = store
                .get_placement("service")?
                .unwrap()
                .replicas
                .into_iter()
                .map(|replica| replica.process_id.context("old worker PID"))
                .collect::<Result<_>>()?;
            config["revision"] = serde_json::json!("revision-2");
            let next = store.upsert_placement("service", &config, DesiredState::Running)?;
            wait_for("uneven previous-cohort drain", || {
                Ok(fixture.directory.path().join("fast-exited").try_exists()?
                    && fixture
                        .directory
                        .path()
                        .join("slow-draining")
                        .try_exists()?)
            })
            .await?;

            // Hold one old worker across two complete reconciliation ticks. A free slot
            // must not admit the new revision while that worker owns the shared data.
            let held = tokio::time::Instant::now();
            while held.elapsed() < Duration::from_millis(2100) {
                ensure!(
                    fixture.starts()? == 2,
                    "new revision started during old-cohort drain"
                );
                ensure!(
                    !fixture.directory.path().join("overlap").try_exists()?,
                    "new and previous revisions ran concurrently"
                );
                ensure!(unsafe { libc::kill(old_pids[1] as i32, 0) } == 0);
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            std::fs::write(&release, [])?;
            wait_for("complete replacement cohort", || {
                let current = store.get_placement("service")?.unwrap();
                Ok(fixture.starts()? == 4
                    && current.running_replicas == 2
                    && current.replicas.iter().all(|replica| {
                        replica.config_revision == next.config_revision
                            && replica.process_id.is_some()
                    }))
            })
            .await?;
            ensure!(!fixture.directory.path().join("overlap").try_exists()?);
            for pid in old_pids {
                assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
            }
            Ok(())
        })
        .await?;
    assert_eq!(store.get_placement("service")?.unwrap().running_replicas, 0);
    Ok(())
}
