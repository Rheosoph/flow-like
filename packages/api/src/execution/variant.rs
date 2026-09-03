//! Live-variant (canary) split resolution for dispatch sites.
//!
//! A [`SplitKey`] is the stickiest identity a trigger surface can offer (D4
//! precedence); [`resolve_live_variant`] deterministically maps it onto the
//! event's Live variants by hashing it into the unit interval and walking the
//! cumulative clamped weights in variant-name order. Shadow variants are never
//! selected here. Page-target events resolve to the primary on the dispatch
//! split; their canary is assigned once at bootstrap ([`resolve_page_target`])
//! and the sealed `page_execution` claims then pin the session to it (WP6b).

use crate::entity::sea_orm_active_enums::RunVariant;
use axum::http::HeaderMap;
use flow_like::flow::event::{Event, EventVariant, EventVariantMode};
use flow_like::flow::variable::Variable;
use std::collections::HashMap;

pub const SPLIT_SOURCE_PIN: &str = "pin";
pub const SPLIT_SOURCE_IDEMPOTENCY_KEY: &str = "idempotency-key";
pub const SPLIT_SOURCE_TRACE: &str = "trace";
pub const SPLIT_SOURCE_SUBJECT: &str = "subject";
pub const SPLIT_SOURCE_RUN_ID: &str = "run-id";
/// Inbound-only split identities: the caller's remote address, and an MCP
/// session id supplied without a variant prefix.
pub const SPLIT_SOURCE_REMOTE_ADDR: &str = "remote-addr";
pub const SPLIT_SOURCE_SESSION: &str = "session";

/// Header carrying an explicit variant pin on the invoke surfaces.
pub const VARIANT_PIN_HEADER: &str = "x-flow-like-variant";

/// The reserved variant name for the primary target's inbound surface —
/// registration buckets, `EventSetup` pointer rows and setup requests all
/// address the primary under this name.
pub const STABLE_VARIANT: &str = "stable";

/// Explicit variant pin from the `x-flow-like-variant` header, falling back to
/// the `__variant` query parameter. Not validated here — the surface decides
/// which names are legal for it.
pub fn pin_from_request(headers: &HeaderMap, query_variant: Option<String>) -> Option<String> {
    headers
        .get(VARIANT_PIN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or(query_variant)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Salted so the assignment keyspace can be rotated deliberately, never by
/// accident.
const VARIANT_SALT: &str = "variant-salt-v1";

/// EventBridge's backward-compatible fallback for schedules created before the
/// target input carried the Scheduler context fields. A Lambda request id is
/// per *invocation*, not per occurrence, so keying on it would hand every
/// retry a different variant on the one deployment that actually retries.
const LAMBDA_REQUEST_KEY_PREFIX: &str = "lambda-request:";

/// The identity a variant assignment is keyed on, plus which precedence level
/// produced it. A `pin` key carries a variant *name* and bypasses hashing.
#[derive(Debug, Clone)]
pub struct SplitKey {
    pub source: &'static str,
    pub value: String,
}

impl SplitKey {
    pub fn pin(variant_name: impl Into<String>) -> Self {
        SplitKey {
            source: SPLIT_SOURCE_PIN,
            value: variant_name.into(),
        }
    }
}

/// The candidate identities one dispatch site can offer, strongest first.
/// Fields a surface cannot produce stay `None` — see D4's unreachability
/// table for why the sink path offers only the idempotency key.
pub struct SplitKeyRequest<'a> {
    /// An already-validated Live variant name (header/query pin).
    pub pinned_variant: Option<&'a str>,
    /// The sink `Idempotency-Key`; `lambda-request:` keys are refused here.
    pub idempotency_key: Option<&'a str>,
    /// Gates the trace level: only an *inherited* trace counts, because every
    /// root dispatch self-roots its trace id from the fresh run id.
    pub parent_run_id: Option<&'a str>,
    pub trace_id: Option<&'a str>,
    /// The authenticated caller (`permission.effective_user_id()`), never a
    /// synthesized `sink:{id}` subject — that is constant per event and would
    /// pin the whole event to one variant forever.
    pub caller_subject: Option<&'a str>,
    /// Last resort, explicitly non-sticky: a fresh draw per fire.
    pub run_id: &'a str,
}

pub fn resolve_split_key(request: &SplitKeyRequest) -> SplitKey {
    if let Some(name) = request.pinned_variant {
        return SplitKey::pin(name);
    }
    if let Some(key) = request
        .idempotency_key
        .filter(|key| !key.starts_with(LAMBDA_REQUEST_KEY_PREFIX))
    {
        return SplitKey {
            source: SPLIT_SOURCE_IDEMPOTENCY_KEY,
            value: key.to_string(),
        };
    }
    if let Some(parent_run_id) = request.parent_run_id {
        return SplitKey {
            source: SPLIT_SOURCE_TRACE,
            value: request.trace_id.unwrap_or(parent_run_id).to_string(),
        };
    }
    if let Some(subject) = request.caller_subject {
        return SplitKey {
            source: SPLIT_SOURCE_SUBJECT,
            value: subject.to_string(),
        };
    }
    SplitKey {
        source: SPLIT_SOURCE_RUN_ID,
        value: request.run_id.to_string(),
    }
}

/// Map a caller-supplied source label (the explain endpoint) onto the static
/// sources. Only `pin` changes behavior — every other source hashes the same.
pub fn static_split_source(source: Option<&str>) -> &'static str {
    match source {
        Some(SPLIT_SOURCE_PIN) => SPLIT_SOURCE_PIN,
        Some(SPLIT_SOURCE_IDEMPOTENCY_KEY) => SPLIT_SOURCE_IDEMPOTENCY_KEY,
        Some(SPLIT_SOURCE_TRACE) => SPLIT_SOURCE_TRACE,
        Some(SPLIT_SOURCE_SUBJECT) => SPLIT_SOURCE_SUBJECT,
        Some(SPLIT_SOURCE_RUN_ID) => SPLIT_SOURCE_RUN_ID,
        Some(SPLIT_SOURCE_REMOTE_ADDR) => SPLIT_SOURCE_REMOTE_ADDR,
        Some(SPLIT_SOURCE_SESSION) => SPLIT_SOURCE_SESSION,
        _ => "explain",
    }
}

/// The dispatch target a split key resolved to. For the primary the overlay is
/// empty and `variant_name` is `None`.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub variant_name: Option<String>,
    pub board_id: String,
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: String,
    /// The page this target renders; `None` for every non-page target.
    pub default_page_id: Option<String>,
    /// Merged over the event's variables per key at dispatch, variant wins;
    /// an empty overlay is pure inheritance.
    pub variables_overlay: HashMap<String, Variable>,
}

impl ResolvedTarget {
    pub fn primary(event: &Event) -> Self {
        ResolvedTarget {
            variant_name: None,
            board_id: event.board_id.clone(),
            board_version: event.board_version,
            node_id: event.node_id.clone(),
            default_page_id: event.default_page_id.clone(),
            variables_overlay: HashMap::new(),
        }
    }

    /// Target an already-validated variant directly, bypassing the split —
    /// used where a variant is addressed by name (e.g. a variant setup run).
    pub fn from_variant(variant: &EventVariant) -> Self {
        ResolvedTarget {
            variant_name: Some(variant.name.clone()),
            board_id: variant.board_id.clone(),
            board_version: variant.board_version,
            node_id: variant.node_id.clone(),
            default_page_id: variant.default_page_id.clone(),
            variables_overlay: variant.variables.clone(),
        }
    }

    /// The run-row tag for this assignment.
    pub fn run_variant(&self) -> RunVariant {
        if self.variant_name.is_some() {
            RunVariant::Canary
        } else {
            RunVariant::Primary
        }
    }
}

/// The assignment plus the `[lo, hi)` slice of the unit interval its target
/// owns — the support-tool view served by the canary explain endpoint.
#[derive(Debug, Clone)]
pub struct VariantAssignment {
    pub variant_name: Option<String>,
    pub share_bounds: (f64, f64),
}

fn live_variants(event: &Event) -> Vec<EventVariant> {
    let mut live: Vec<EventVariant> = event
        .variant_set()
        .into_iter()
        .filter(|variant| matches!(variant.mode, EventVariantMode::Live { .. }))
        .collect();
    live.sort_by(|a, b| a.name.cmp(&b.name));
    live
}

/// blake3(event_id ‖ salt ‖ key) → first 8 bytes → `[0, 1)`.
///
/// The event version is deliberately NOT part of the hash: every content edit
/// bumps it, so including it would reshuffle every caller's assignment on an
/// unrelated description change — noise in the canary comparison, and for page
/// canaries a viewer flipping variants between bootstraps while their sealed
/// session claims still point at the old one. Assignments therefore move only
/// when the variant set or weights change, which is the operator's intent.
/// Nothing random, no shared state, identical on every replica.
fn unit_point(event: &Event, key_value: &str) -> f64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(event.id.as_bytes());
    hasher.update(&[0]);
    hasher.update(VARIANT_SALT.as_bytes());
    hasher.update(&[0]);
    hasher.update(key_value.as_bytes());
    let digest = hasher.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest.as_bytes()[..8]);
    // 2^64 is exact in f64, so the quotient stays strictly below 1.0.
    (u64::from_be_bytes(first) as f64) / (u64::MAX as f64 + 1.0)
}

fn assign(event: &Event, key: &SplitKey) -> (Option<EventVariant>, (f64, f64)) {
    // Page-event canaries are bootstrap-resolved (WP6b): the dispatch split
    // always serves the primary for an event that owns a page.
    if event.default_page_id.is_some() {
        return (None, (0.0, 1.0));
    }
    assign_live(event, key)
}

fn assign_live(event: &Event, key: &SplitKey) -> (Option<EventVariant>, (f64, f64)) {
    let point = (key.source != SPLIT_SOURCE_PIN).then(|| unit_point(event, &key.value));
    let mut cursor = 0.0_f64;
    let mut selected: Option<(EventVariant, (f64, f64))> = None;
    for variant in live_variants(event) {
        let share = f64::from(variant.mode.share());
        let upper = (cursor + share).min(1.0);
        let chosen = match point {
            // Half-open slices: a zero-weight variant is empty and therefore
            // reachable only through an explicit pin.
            Some(point) => point >= cursor && point < upper,
            None => variant.name == key.value,
        };
        if chosen && selected.is_none() {
            selected = Some((variant, (cursor, upper)));
        }
        cursor = upper;
    }
    match selected {
        Some((variant, bounds)) => (Some(variant), bounds),
        None => (None, (cursor, 1.0)),
    }
}

/// Deterministic Live-variant assignment for one split key. Shadow variants
/// are never selected; an unknown pin falls back to the primary (the invoke
/// surfaces validate pins before resolving).
pub fn resolve_live_variant(event: &Event, key: &SplitKey) -> ResolvedTarget {
    match assign(event, key).0 {
        Some(variant) => ResolvedTarget::from_variant(&variant),
        None => ResolvedTarget::primary(event),
    }
}

/// Recompute any past or hypothetical assignment — the canary explain view.
pub fn explain(event: &Event, key: &SplitKey) -> VariantAssignment {
    let (variant, share_bounds) = assign(event, key);
    VariantAssignment {
        variant_name: variant.map(|variant| variant.name),
        share_bounds,
    }
}

/// What a request selected explicitly, before any hashing.
pub enum ExplicitSelection {
    /// No explicit selector — assign by the hashed split key.
    Hashed,
    /// The request named the primary's exact board version: today's contract
    /// is that this runs the primary, so it must never hash into a variant.
    Primary,
    Variant(String),
}

/// Level-1 explicit selection for the invoke surfaces: a named pin
/// (`x-flow-like-variant` header / `__variant` query) must be a Live variant;
/// a request `version` equal to a Live variant's pinned board version selects
/// that variant, while the primary's own version keeps the primary.
pub fn explicit_selection(
    event: &Event,
    named_pin: Option<&str>,
    requested_version: Option<(u32, u32, u32)>,
) -> Result<ExplicitSelection, String> {
    if let Some(name) = named_pin.map(str::trim).filter(|name| !name.is_empty()) {
        if !live_variants(event)
            .iter()
            .any(|variant| variant.name == name)
        {
            return Err(format!(
                "'{name}' is not a live variant of this event; only live variants can be pinned"
            ));
        }
        return Ok(ExplicitSelection::Variant(name.to_string()));
    }
    if let Some(requested) = requested_version {
        if event.board_version == Some(requested) {
            return Ok(ExplicitSelection::Primary);
        }
        if let Some(variant) = live_variants(event)
            .into_iter()
            .find(|variant| variant.board_version == Some(requested))
        {
            return Ok(ExplicitSelection::Variant(variant.name));
        }
    }
    Ok(ExplicitSelection::Hashed)
}

/// One-call resolution for the invoke surfaces: explicit selection first, else
/// the hashed split key. `Err` names an unknown or non-Live pin (a 400 at the
/// route).
pub fn resolve_invoke_target(
    event: &Event,
    named_pin: Option<&str>,
    requested_version: Option<(u32, u32, u32)>,
    fallback: &SplitKeyRequest,
) -> Result<ResolvedTarget, String> {
    match explicit_selection(event, named_pin, requested_version)? {
        ExplicitSelection::Primary => Ok(ResolvedTarget::primary(event)),
        ExplicitSelection::Variant(name) => Ok(resolve_live_variant(event, &SplitKey::pin(name))),
        ExplicitSelection::Hashed => Ok(resolve_live_variant(event, &resolve_split_key(fallback))),
    }
}

/// Assignment for a root sink fire: the occurrence identity when the caller
/// supplied one (stable across retries of the same occurrence), else an honest
/// per-fire draw on the run id. Every richer level of the precedence is
/// unreachable on the sink path (D4).
pub fn resolve_sink_target(
    event: &Event,
    idempotency_key: Option<&str>,
    run_id: &str,
) -> ResolvedTarget {
    let key = resolve_split_key(&SplitKeyRequest {
        pinned_variant: None,
        idempotency_key,
        parent_run_id: None,
        trace_id: None,
        caller_subject: None,
        run_id,
    });
    resolve_live_variant(event, &key)
}

/// The legal render targets of a page event: the primary plus every Live
/// variant that names its own page. A Live variant without a page (only a
/// grandfathered legacy `canary` can be one) has nothing to render and is not
/// a page target. Empty for an event that owns no page.
pub fn page_targets(event: &Event) -> Vec<ResolvedTarget> {
    if event.default_page_id.is_none() {
        return Vec::new();
    }
    std::iter::once(ResolvedTarget::primary(event))
        .chain(
            live_variants(event)
                .iter()
                .filter(|variant| variant.default_page_id.is_some())
                .map(ResolvedTarget::from_variant),
        )
        .collect()
}

/// An explicit page-target selection: [`STABLE_VARIANT`] names the primary, a
/// Live variant name selects that variant. `Ok(None)` when nothing was pinned;
/// `Err` names an unknown or non-Live pin, or a Live variant without a page.
pub fn explicit_page_target(
    event: &Event,
    named_pin: Option<&str>,
) -> Result<Option<ResolvedTarget>, String> {
    let Some(pin) = named_pin.map(str::trim).filter(|pin| !pin.is_empty()) else {
        return Ok(None);
    };
    if pin == STABLE_VARIANT {
        return Ok(Some(ResolvedTarget::primary(event)));
    }
    match explicit_selection(event, Some(pin), None)? {
        ExplicitSelection::Variant(name) => page_targets(event)
            .into_iter()
            .find(|target| target.variant_name.as_deref() == Some(name.as_str()))
            .map(Some)
            .ok_or_else(|| format!("variant '{name}' does not name a page and cannot be served")),
        ExplicitSelection::Primary | ExplicitSelection::Hashed => Ok(None),
    }
}

/// Bootstrap-time page-event assignment (WP6b): an explicit pin wins, else the
/// split key is hashed over the Live variants exactly like the dispatch split.
/// A hashed hit on a page-less variant serves the primary. Non-page events
/// always resolve to the primary here — their canary is a dispatch concern.
pub fn resolve_page_target(
    event: &Event,
    named_pin: Option<&str>,
    split_key: &SplitKey,
) -> Result<ResolvedTarget, String> {
    if let Some(target) = explicit_page_target(event, named_pin)? {
        return Ok(target);
    }
    if event.default_page_id.is_none() {
        return Ok(ResolvedTarget::primary(event));
    }
    Ok(match assign_live(event, split_key).0 {
        Some(variant) if variant.default_page_id.is_some() => {
            ResolvedTarget::from_variant(&variant)
        }
        _ => ResolvedTarget::primary(event),
    })
}

/// The event as the target sees it: unchanged for the primary; for a variant,
/// the variant's board, version, node and page swapped in and its variables
/// merged over the event's (variant wins per key). Never persisted — the
/// stored event keeps the primary target.
pub fn apply_target(mut event: Event, target: &ResolvedTarget) -> Event {
    if target.variant_name.is_none() {
        return event;
    }
    event.board_id = target.board_id.clone();
    event.board_version = target.board_version;
    event.node_id = target.node_id.clone();
    event.default_page_id = target.default_page_id.clone();
    for (name, variable) in &target.variables_overlay {
        event.variables.insert(name.clone(), variable.clone());
    }
    event
}

/// The event the executor sees for this run — see [`apply_target`].
pub fn dispatch_event_json(event: &Event, target: &ResolvedTarget) -> serde_json::Result<String> {
    if target.variant_name.is_none() {
        return serde_json::to_string(event);
    }
    serde_json::to_string(&apply_target(event.clone(), target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::event::CanaryEvent;

    fn test_event() -> Event {
        Event {
            id: "event-1".to_string(),
            name: "Test".to_string(),
            description: String::new(),
            board_id: "board-primary".to_string(),
            board_version: Some((2, 0, 0)),
            node_id: "node-primary".to_string(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "generic".to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    fn live_variant(name: &str, weight: f32) -> EventVariant {
        EventVariant {
            name: name.to_string(),
            board_id: "board-canary".to_string(),
            board_version: Some((3, 0, 0)),
            node_id: "node-canary".to_string(),
            variables: HashMap::new(),
            default_page_id: None,
            mode: EventVariantMode::Live { weight },
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    fn subject_key(value: &str) -> SplitKey {
        SplitKey {
            source: SPLIT_SOURCE_SUBJECT,
            value: value.to_string(),
        }
    }

    #[test]
    fn assignment_is_deterministic() {
        let mut event = test_event();
        event.variants = vec![live_variant("canary", 0.5)];

        let first = resolve_live_variant(&event, &subject_key("user-42"));
        for _ in 0..100 {
            let again = resolve_live_variant(&event, &subject_key("user-42"));
            assert_eq!(again.variant_name, first.variant_name);
            assert_eq!(again.board_id, first.board_id);
        }
        let explained = explain(&event, &subject_key("user-42"));
        assert_eq!(explained.variant_name, first.variant_name);
    }

    #[test]
    fn distribution_over_many_keys_matches_the_weight() {
        let mut event = test_event();
        event.variants = vec![live_variant("canary", 0.3)];

        let total = 10_000;
        let hits = (0..total)
            .filter(|i| {
                resolve_live_variant(&event, &subject_key(&format!("key-{i}")))
                    .variant_name
                    .is_some()
            })
            .count();
        let share = hits as f64 / total as f64;
        assert!(
            (share - 0.3).abs() <= 0.02,
            "canary served {share} of 10k keys, expected 0.30 ± 0.02"
        );
    }

    #[test]
    fn weight_zero_is_reachable_only_through_a_pin() {
        let mut event = test_event();
        event.variants = vec![live_variant("dark", 0.0)];

        for i in 0..2_000 {
            let target = resolve_live_variant(&event, &subject_key(&format!("key-{i}")));
            assert_eq!(target.variant_name, None);
        }

        let pinned = resolve_live_variant(&event, &SplitKey::pin("dark"));
        assert_eq!(pinned.variant_name.as_deref(), Some("dark"));
        assert_eq!(pinned.board_id, "board-canary");

        let explained = explain(&event, &SplitKey::pin("dark"));
        assert_eq!(explained.share_bounds, (0.0, 0.0));
    }

    #[test]
    fn page_events_always_resolve_to_the_primary() {
        let mut event = test_event();
        event.default_page_id = Some("page-1".to_string());
        event.variants = vec![live_variant("canary", 1.0)];

        for i in 0..100 {
            let target = resolve_live_variant(&event, &subject_key(&format!("key-{i}")));
            assert_eq!(target.variant_name, None);
            assert_eq!(target.board_id, "board-primary");
        }
        let explained = explain(&event, &subject_key("any"));
        assert_eq!(explained.share_bounds, (0.0, 1.0));
    }

    #[test]
    fn page_targets_are_the_primary_plus_every_live_page_variant() {
        let mut event = test_event();
        event.default_page_id = Some("page-1".to_string());
        let mut paged = live_variant("canary", 0.5);
        paged.default_page_id = Some("page-canary".to_string());
        let pageless = live_variant("legacy", 0.5);
        let mut shadow = live_variant("mirror", 0.5);
        shadow.default_page_id = Some("page-shadow".to_string());
        shadow.mode = EventVariantMode::Shadow { sample_rate: 0.5 };
        event.variants = vec![paged, pageless, shadow];

        let targets = page_targets(&event);
        let names: Vec<Option<&str>> = targets
            .iter()
            .map(|target| target.variant_name.as_deref())
            .collect();
        assert_eq!(names, vec![None, Some("canary")]);
        assert!(page_targets(&test_event()).is_empty());
    }

    #[test]
    fn page_target_resolution_pins_hashes_and_refuses_unknown_pins() {
        let mut event = test_event();
        event.default_page_id = Some("page-1".to_string());
        let mut paged = live_variant("canary", 1.0);
        paged.default_page_id = Some("page-canary".to_string());
        event.variants = vec![paged];

        let hashed = resolve_page_target(&event, None, &subject_key("user-1")).unwrap();
        assert_eq!(hashed.variant_name.as_deref(), Some("canary"));
        assert_eq!(hashed.default_page_id.as_deref(), Some("page-canary"));
        assert_eq!(hashed.board_id, "board-canary");

        let stable =
            resolve_page_target(&event, Some(STABLE_VARIANT), &subject_key("user-1")).unwrap();
        assert_eq!(stable.variant_name, None);
        assert_eq!(stable.default_page_id.as_deref(), Some("page-1"));

        assert!(resolve_page_target(&event, Some("nope"), &subject_key("user-1")).is_err());

        // A hashed hit on a page-less (legacy) variant serves the primary.
        event.variants = vec![live_variant("legacy", 1.0)];
        let fallback = resolve_page_target(&event, None, &subject_key("user-1")).unwrap();
        assert_eq!(fallback.variant_name, None);
        assert!(resolve_page_target(&event, Some("legacy"), &subject_key("user-1")).is_err());

        let applied = apply_target(event.clone(), &hashed);
        assert_eq!(applied.board_id, "board-canary");
        assert_eq!(applied.board_version, Some((3, 0, 0)));
        assert_eq!(applied.default_page_id.as_deref(), Some("page-canary"));
        assert_eq!(event.default_page_id.as_deref(), Some("page-1"));
    }

    #[test]
    fn legacy_canary_serves_through_variant_set_fallback() {
        let mut event = test_event();
        event.canary = Some(CanaryEvent {
            weight: 1.0,
            variables: HashMap::new(),
            board_id: "board-legacy".to_string(),
            board_version: None,
            node_id: "node-legacy".to_string(),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        });

        let target = resolve_live_variant(&event, &subject_key("anyone"));
        assert_eq!(target.variant_name.as_deref(), Some("canary"));
        assert_eq!(target.board_id, "board-legacy");
        assert_eq!(target.board_version, None);
        assert_eq!(target.node_id, "node-legacy");
    }

    #[test]
    fn shadow_variants_are_never_selected() {
        let mut event = test_event();
        let mut shadow = live_variant("shadow", 1.0);
        shadow.mode = EventVariantMode::Shadow { sample_rate: 1.0 };
        event.variants = vec![shadow];

        for i in 0..100 {
            let target = resolve_live_variant(&event, &subject_key(&format!("key-{i}")));
            assert_eq!(target.variant_name, None);
        }
        let pinned = resolve_live_variant(&event, &SplitKey::pin("shadow"));
        assert_eq!(pinned.variant_name, None);
    }

    #[test]
    fn split_key_precedence_follows_d4() {
        let base = |run_id: &'static str| SplitKeyRequest {
            pinned_variant: None,
            idempotency_key: None,
            parent_run_id: None,
            trace_id: None,
            caller_subject: None,
            run_id,
        };

        let key = resolve_split_key(&SplitKeyRequest {
            pinned_variant: Some("canary"),
            idempotency_key: Some("occ-1"),
            caller_subject: Some("user-1"),
            ..base("run-1")
        });
        assert_eq!((key.source, key.value.as_str()), ("pin", "canary"));

        let key = resolve_split_key(&SplitKeyRequest {
            idempotency_key: Some("occ-1"),
            parent_run_id: Some("parent-1"),
            trace_id: Some("trace-1"),
            caller_subject: Some("user-1"),
            ..base("run-1")
        });
        assert_eq!(
            (key.source, key.value.as_str()),
            ("idempotency-key", "occ-1")
        );

        let key = resolve_split_key(&SplitKeyRequest {
            parent_run_id: Some("parent-1"),
            trace_id: Some("trace-1"),
            caller_subject: Some("user-1"),
            ..base("run-1")
        });
        assert_eq!((key.source, key.value.as_str()), ("trace", "trace-1"));

        let key = resolve_split_key(&SplitKeyRequest {
            caller_subject: Some("user-1"),
            ..base("run-1")
        });
        assert_eq!((key.source, key.value.as_str()), ("subject", "user-1"));

        let key = resolve_split_key(&base("run-1"));
        assert_eq!((key.source, key.value.as_str()), ("run-id", "run-1"));
    }

    #[test]
    fn lambda_request_idempotency_keys_are_refused() {
        let key = resolve_split_key(&SplitKeyRequest {
            pinned_variant: None,
            idempotency_key: Some("lambda-request:abc-123"),
            parent_run_id: None,
            trace_id: None,
            caller_subject: None,
            run_id: "run-1",
        });
        assert_eq!((key.source, key.value.as_str()), ("run-id", "run-1"));
    }

    #[test]
    fn a_self_rooted_trace_never_counts() {
        // Every root dispatch sets trace_id = run_id; only an inherited trace
        // (parent_run_id present) is sticky.
        let key = resolve_split_key(&SplitKeyRequest {
            pinned_variant: None,
            idempotency_key: None,
            parent_run_id: None,
            trace_id: Some("run-1"),
            caller_subject: Some("user-1"),
            run_id: "run-1",
        });
        assert_eq!((key.source, key.value.as_str()), ("subject", "user-1"));
    }

    #[test]
    fn requested_version_selects_primary_or_pinned_variant() {
        let mut event = test_event();
        event.variants = vec![live_variant("canary", 0.5)];

        assert!(matches!(
            explicit_selection(&event, None, Some((2, 0, 0))),
            Ok(ExplicitSelection::Primary)
        ));
        match explicit_selection(&event, None, Some((3, 0, 0))) {
            Ok(ExplicitSelection::Variant(name)) => assert_eq!(name, "canary"),
            _ => panic!("a variant's pinned version must select that variant"),
        }
        assert!(matches!(
            explicit_selection(&event, None, Some((9, 9, 9))),
            Ok(ExplicitSelection::Hashed)
        ));
        assert!(explicit_selection(&event, Some("nope"), None).is_err());

        let mut shadow = live_variant("mirror", 0.5);
        shadow.mode = EventVariantMode::Shadow { sample_rate: 0.5 };
        event.variants.push(shadow);
        assert!(explicit_selection(&event, Some("mirror"), None).is_err());
    }

    #[test]
    fn dispatch_event_json_overlays_only_for_variants() {
        use flow_like::flow::pin::ValueType;
        use flow_like::flow::variable::VariableType;

        let test_variable =
            |name: &str| Variable::new(name, VariableType::String, ValueType::Normal);
        let mut event = test_event();
        event
            .variables
            .insert("shared".to_string(), test_variable("shared"));
        let mut variant = live_variant("canary", 1.0);
        variant
            .variables
            .insert("extra".to_string(), test_variable("extra"));
        event.variants = vec![variant];

        let primary = ResolvedTarget::primary(&event);
        let primary_json = dispatch_event_json(&event, &primary).unwrap();
        assert_eq!(primary_json, serde_json::to_string(&event).unwrap());

        let target = resolve_live_variant(&event, &SplitKey::pin("canary"));
        let json = dispatch_event_json(&event, &target).unwrap();
        let overlaid: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(overlaid.board_id, "board-canary");
        assert_eq!(overlaid.board_version, Some((3, 0, 0)));
        assert_eq!(overlaid.node_id, "node-canary");
        assert!(overlaid.variables.contains_key("shared"));
        assert!(overlaid.variables.contains_key("extra"));
        // The stored event is untouched.
        assert_eq!(event.board_id, "board-primary");
        assert!(!event.variables.contains_key("extra"));
    }
}
