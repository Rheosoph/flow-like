use super::PreparedInvocation;
use anyhow::{Context, Result, ensure};
use flow_like_runtime::{
    a2ui::element_demand,
    flow::compiled::prerun::{
        PrerunPageExecution, decorate_page_actions, page_execution_revision,
        redact_page_execution_routes,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) struct PreparedPage {
    pub invocation: PreparedInvocation,
    pub bootstrap: Value,
    revision: String,
    execution: PrerunPageExecution,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInvocation {
    trigger: PageTrigger,
    manifest_revision: String,
    #[serde(default)]
    payload: Option<Value>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PageTrigger {
    Action {
        action_id: String,
        #[serde(default)]
        capability_jwt: Option<String>,
    },
    Special {
        special_event: SpecialEvent,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SpecialEvent {
    Load,
    Unload,
    Interval,
}

impl PreparedPage {
    pub(super) async fn load(
        placement_id: &str,
        project_id: &str,
        invocation: PreparedInvocation,
        state: &flow_like_runtime::state::FlowLikeState,
    ) -> Result<Self> {
        let event = &invocation.event;
        let page_id = event
            .default_page_id
            .as_deref()
            .context("Page Event has no Page")?;
        flow_like_device_protocol::validate_instance_identifier(page_id)?;
        let board = &invocation.template.board;
        ensure!(
            event.board_version.is_some() && event.board_version == Some(board.version),
            "Page Event requires its exact pinned Board version"
        );
        ensure!(
            board.page_ids.iter().any(|id| id == page_id),
            "Page is absent from the pinned Board"
        );
        let store = state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .context("Project metadata store is unavailable")?
            .as_generic();
        let page = board
            .load_versioned_page(page_id, board.version, Some(store))
            .await?;
        ensure!(
            page.id == page_id && page.board_id.as_deref().is_none_or(|id| id == board.id),
            "Pinned Page identity differs from its Event"
        );
        let execution = PrerunPageExecution::from_page(board, &page)?;
        let contract_revision = page_execution_revision(board, &execution)?;
        let revision = format!(
            "spr1_{}",
            blake3::hash(&serde_json::to_vec(&(
                placement_id,
                project_id,
                &event.id,
                event.event_version,
                &contract_revision,
            ))?)
            .to_hex()
        );
        for node in execution
            .action_events
            .iter()
            .map(|a| a.node_id.as_str())
            .chain(
                execution
                    .special_events
                    .load
                    .iter()
                    .map(|a| a.node_id.as_str()),
            )
            .chain(
                execution
                    .special_events
                    .unload
                    .iter()
                    .map(|a| a.node_id.as_str()),
            )
            .chain(
                execution
                    .special_events
                    .interval
                    .iter()
                    .map(|a| a.node_id.as_str()),
            )
        {
            ensure!(
                invocation
                    .template
                    .nodes
                    .iter()
                    .any(|n| n.id.as_ref() == node && n.can_seed_run),
                "Page action does not resolve to an executable entry"
            );
        }
        let page =
            redact_page_execution_routes(&decorate_page_actions(&page, &execution, &revision)?)?;
        let bootstrap = json!({
            "project_id": project_id, "event_id": event.id, "name": event.name,
            "page": page, "execution_revision": revision,
            "element_demand": element_demand(board),
            "route": event.route,
            "event": super::public_event(event),
        });
        ensure!(
            serde_json::to_vec(&bootstrap)?.len() <= super::BODY_LIMIT,
            "Page exceeds the service response limit"
        );
        Ok(Self {
            invocation,
            bootstrap,
            revision,
            execution,
        })
    }

    pub(super) fn scope(&self, project_id: &str) -> super::actions::Scope {
        let event = &self.invocation.event;
        let version = event.board_version.expect("Page Board is pinned");
        super::actions::Scope {
            app_id: project_id.into(),
            event_id: event.id.clone(),
            page_id: event.default_page_id.clone().expect("Page Event"),
            board_id: event.board_id.clone(),
            board_version: [version.0, version.1, version.2],
            manifest_revision: self.revision.clone(),
        }
    }

    pub(super) async fn select(
        &self,
        host: &super::HostState,
        body: &[u8],
        fingerprint: blake3::Hash,
    ) -> Result<(PreparedInvocation, Option<Value>)> {
        let request: PageInvocation = serde_json::from_slice(body)?;
        ensure!(
            request.manifest_revision == self.revision,
            "Page manifest has changed"
        );
        let dynamic;
        let mut admission = None;
        let node = match request.trigger {
            PageTrigger::Action {
                action_id,
                capability_jwt: Some(capability),
            } => {
                ensure!(
                    action_id.starts_with("da1_") && action_id.len() <= 128,
                    "Invalid dynamic Page action"
                );
                dynamic = super::actions::resolve(
                    host,
                    &self.scope(&host.project_id),
                    &action_id,
                    &capability,
                    fingerprint,
                )
                .await?;
                admission = Some(super::actions::Admission {
                    scope: self.scope(&host.project_id),
                    action_id,
                    capability,
                });
                ensure!(
                    self.invocation
                        .template
                        .nodes
                        .iter()
                        .any(|node| node.id.as_ref() == dynamic && node.can_seed_run),
                    "Page callback entry is unavailable"
                );
                Some(dynamic.as_str())
            }
            PageTrigger::Action {
                action_id,
                capability_jwt: None,
            } => self
                .execution
                .action_events
                .iter()
                .find(|a| a.action_id == action_id)
                .map(|a| a.node_id.as_str()),
            PageTrigger::Special {
                special_event: SpecialEvent::Load,
            } => self
                .execution
                .special_events
                .load
                .as_ref()
                .map(|a| a.node_id.as_str()),
            PageTrigger::Special {
                special_event: SpecialEvent::Unload,
            } => self
                .execution
                .special_events
                .unload
                .as_ref()
                .map(|a| a.node_id.as_str()),
            PageTrigger::Special {
                special_event: SpecialEvent::Interval,
            } => self
                .execution
                .special_events
                .interval
                .as_ref()
                .map(|a| a.node_id.as_str()),
        }
        .context("Page action is not configured")?;
        let mut invocation = self.invocation.clone();
        invocation.event.node_id = node.into();
        invocation.action_admission = admission;
        Ok((invocation, request.payload))
    }
}
