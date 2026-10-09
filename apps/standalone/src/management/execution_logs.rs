use super::*;

const MAX_OFFSET: u32 = 1_000_000;

#[cfg(any(feature = "runtime", test))]
fn continuation(offset: u32, length: usize, more: bool) -> (Option<u32>, bool) {
    let next = more.then_some(offset.saturating_add(length as u32));
    (
        next.filter(|next| *next <= MAX_OFFSET),
        next.is_some_and(|next| next > MAX_OFFSET),
    )
}

#[cfg(any(feature = "runtime", test))]
fn bounded_message_row(mut row: Value, message: &str) -> Result<Value> {
    row["message"] = json!("");
    row["truncated"] = json!(false);
    // Escaped quotes and control characters can outgrow the raw UTF-8 byte cap.
    // Reserve the page envelope and comma, then fit the serialized string itself.
    let overhead = serde_json::to_vec(&row)?.len() - 2;
    let budget = (noise::MAX_PLAINTEXT - 1025)
        .checked_sub(overhead)
        .context("Execution message metadata exceeds the encrypted message limit")?;
    let boundaries: Vec<usize> = message
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= 8192)
        .chain((message.len() <= 8192).then_some(message.len()))
        .collect();
    let (mut low, mut high) = (0, boundaries.len());
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if serde_json::to_vec(&message[..boundaries[middle]])?.len() <= budget {
            low = middle;
        } else {
            high = middle;
        }
    }
    let end = boundaries[low];
    row["message"] = json!(&message[..end]);
    row["truncated"] = json!(end < message.len());
    Ok(row)
}

fn validate(command: &ManagementCommand) -> Result<(&str, u32, u16)> {
    let (placement, offset, limit, maximum) = match command {
        ManagementCommand::ExecutionRuns {
            placement_id,
            offset,
            limit,
        } => (placement_id, *offset, *limit, 20),
        ManagementCommand::ExecutionLogs {
            placement_id,
            run_id,
            offset,
            limit,
            node_id,
            min_level,
        } => {
            crate::config::validate_id("run", run_id).reject_as(RejectionCode::Invalid)?;
            if let Some(node) = node_id {
                validate_management_id(node).reject_as(RejectionCode::Invalid)?;
            }
            refuse_unless(
                min_level.is_none_or(|level| level <= 4),
                RejectionCode::Invalid,
                "Log level must be between 0 and 4",
            )?;
            (placement_id, *offset, *limit, 50)
        }
        _ => {
            return Err(refusal(
                RejectionCode::Invalid,
                "Expected an execution log command",
            ));
        }
    };
    crate::config::validate_id("placement", placement).reject_as(RejectionCode::Invalid)?;
    refuse_unless(
        (1..=maximum).contains(&limit) && offset <= MAX_OFFSET,
        RejectionCode::Invalid,
        "Execution log page is outside its bounds",
    )?;
    Ok((placement, offset, limit))
}

pub(super) async fn execute_async(
    service: &Arc<ManagementService>,
    authority: &Authority,
    request: &ManagementRequest,
    now: i64,
) -> Result<ManagementResponse> {
    read(
        &service.state_dir,
        service.device.manifest(),
        authority,
        request,
        now,
    )
    .await
}

pub(super) async fn read(
    state_dir: &Path,
    manifest: &OnboardingManifest,
    authority: &Authority,
    request: &ManagementRequest,
    now: i64,
) -> Result<ManagementResponse> {
    validate_request(request, &manifest.device_id, now)?;
    let (placement_id, _, _) = validate(&request.command)?;
    #[cfg(not(feature = "runtime"))]
    {
        let _ = (state_dir, authority, placement_id);
        Err(refusal(
            RejectionCode::Unsupported,
            "This device agent was built without execution logs",
        ))
    }
    #[cfg(feature = "runtime")]
    {
        let started = std::time::Instant::now();
        let config = {
            let store = StateStore::open(&state_dir.join("management.sqlite"))?;
            authorized_read(
                &store,
                authority.read_guard(
                    manifest,
                    request,
                    now,
                    Some(ManagementCapability::Logs),
                    Some(placement_id),
                ),
                || {
                    let (record, project) = placement_scope(&store, placement_id)?;
                    authority.require(
                        ManagementCapability::Logs,
                        Some(&project),
                        Some(placement_id),
                    )?;
                    Ok(record.config)
                },
            )?
        };
        let placement: crate::config::PlacementConfig = serde_json::from_value(config.clone())?;
        let result = read_logs(state_dir, &placement, &request.command).await?;
        // A revoked grant or replacement placement must never receive an in-flight read.
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let now = now.saturating_add(started.elapsed().as_secs() as i64);
        authorized_read(
            &store,
            authority.read_guard(
                manifest,
                request,
                now,
                Some(ManagementCapability::Logs),
                Some(placement_id),
            ),
            || {
                let (record, _) = placement_scope(&store, placement_id)?;
                refuse_unless(
                    record.config == config,
                    RejectionCode::RevisionConflict,
                    "Placement changed while reading execution logs",
                )?;
                completed(request, "Execution logs", result)
            },
        )
    }
}

#[cfg(feature = "runtime")]
async fn read_logs(
    state_dir: &Path,
    config: &crate::config::PlacementConfig,
    command: &ManagementCommand,
) -> Result<Value> {
    use crate::execution_logs::{ExecutionIndex, existing_child, logs_root};
    use flow_like_runtime::flow::execution::{
        log::StoredLogMessage,
        log_query::{LogQuery, query_log_page},
        run_index::{RunIndex, RunQuery},
    };
    let (placement, offset, limit) = validate(command)?;
    let root = logs_root(state_dir, config)?;
    let index = root
        .as_deref()
        .map(ExecutionIndex::existing)
        .transpose()?
        .flatten();
    let mut rows = Vec::new();
    match command {
        ManagementCommand::ExecutionRuns { .. } => {
            if let Some(index) = index {
                let query = RunQuery {
                    offset: offset as usize,
                    limit: usize::from(limit) + 1,
                    ..RunQuery::new(&config.project_id)
                };
                for meta in index.list(&query).await? {
                    rows.push(json!({"run_id":meta.run_id,"board_id":meta.board_id,"event_id":meta.event_id,
                        "node_id":meta.node_id,"version":meta.version,"event_version":meta.event_version,
                        "start":meta.start,"end":meta.end,"log_level":meta.log_level,"logs":meta.logs}));
                }
            }
            let (rows, more) = page(rows, usize::from(limit), "execution summaries")?;
            let (next, limited) = continuation(offset, rows.len(), more);
            Ok(
                json!({"placement_id":placement,"runs":rows,"next_offset":next,"limit_reached":limited}),
            )
        }
        ManagementCommand::ExecutionLogs {
            run_id,
            node_id,
            min_level,
            ..
        } => {
            let index = index.ok_or_else(|| {
                refusal(
                    RejectionCode::Invalid,
                    "No recorded execution on this service",
                )
            })?;
            let meta = index
                .get(&config.project_id, run_id)
                .await?
                .ok_or_else(|| {
                    refusal(RejectionCode::Invalid, "Unknown execution on this service")
                })?;
            crate::config::validate_id("project", &config.project_id)?;
            crate::config::validate_id("board", &meta.board_id)?;
            let relative = PathBuf::from("runs")
                .join(&config.project_id)
                .join(&meta.board_id);
            let root = root.context("Execution log store is missing")?;
            let database =
                existing_child(&root, &relative)?.context("Execution log database is missing")?;
            existing_child(&database, &PathBuf::from(format!("{run_id}.lance")))?
                .context("Execution log table is missing")?;
            let database = flow_like_storage::databases::vector::lancedb::connect_lance(
                database.to_string_lossy().as_ref(),
            )
            .execute()
            .await?;
            let table = database.open_table(run_id).execute().await?;
            let query = LogQuery {
                nodes: node_id.iter().cloned().collect(),
                levels: min_level
                    .map(|level| (level..=4).collect())
                    .unwrap_or_default(),
                ..Default::default()
            };
            let messages =
                query_log_page(&table, &query, offset as usize, usize::from(limit) + 1).await?;
            for message in messages {
                let log = StoredLogMessage::from(message);
                rows.push(bounded_message_row(
                    json!({"node_id":log.node_id,"operation_id":log.operation_id,
                        "log_level":log.log_level,"start":log.start,"end":log.end}),
                    &log.message,
                )?);
            }
            let (rows, more) = page(rows, usize::from(limit), "execution messages")?;
            let (next, limited) = continuation(offset, rows.len(), more);
            Ok(
                json!({"placement_id":placement,"run_id":run_id,"logs":rows,"next_offset":next,"limit_reached":limited}),
            )
        }
        _ => unreachable!("validated execution command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_log_cursor_never_exceeds_the_allowed_offset() {
        assert_eq!(continuation(999_980, 20, true), (Some(MAX_OFFSET), false));
        assert_eq!(continuation(MAX_OFFSET, 20, true), (None, true));
        assert_eq!(continuation(MAX_OFFSET, 20, false), (None, false));
    }

    #[test]
    fn execution_log_messages_fit_after_json_escaping() -> Result<()> {
        for message in ["\"".repeat(8192), "\0".repeat(8192), "ü".repeat(9000)] {
            let row = bounded_message_row(
                json!({"node_id":"node","operation_id":null,"log_level":1,"start":1,"end":2}),
                &message,
            )?;
            assert_eq!(row["truncated"], true);
            assert!(row["message"].as_str().unwrap().len() <= 8192);
            let (rows, more) = page(vec![row], 50, "execution messages")?;
            assert_eq!(rows.len(), 1);
            assert!(!more);
        }
        let row = bounded_message_row(json!({}), "")?;
        assert_eq!(row["message"], "");
        assert_eq!(row["truncated"], false);
        let row = bounded_message_row(json!({}), "ordinary message")?;
        assert_eq!(row["message"], "ordinary message");
        assert_eq!(row["truncated"], false);
        Ok(())
    }
}
