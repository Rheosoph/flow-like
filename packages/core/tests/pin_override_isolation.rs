//! Isolation guarantees of `ExecutionContext::context_pin_overrides`.
//!
//! The override map is what keeps parallel function invocations and loop iterations from
//! reading each other's pin values. It is cloned once per node execution and merged back
//! on every sub-context pop, so it is also the hottest allocation on a loop-heavy board —
//! which is why its values are shared (`Arc<Value>`) rather than owned. Sharing is only
//! safe because entries are replaced wholesale, never mutated in place; these tests pin
//! that behaviour so a future edit cannot quietly turn sharing into aliasing.

mod support;

use flow_like_types::json::json;
use support::context_with_callback;

#[tokio::test]
async fn a_child_inherits_overrides_but_does_not_leak_back_until_merged() {
    let (mut context, _channel) = context_with_callback("override-inherit", None).await;
    context.override_pin_value("pin-parent", json!("parent"));

    let node = context.node.clone();
    let mut child = context.create_sub_context(&node).await;

    let inherited = child
        .context_pin_overrides
        .as_ref()
        .and_then(|map| map.get("pin-parent"))
        .cloned();
    assert_eq!(
        inherited.as_deref(),
        Some(&json!("parent")),
        "a sub-context must start from the parent's overrides"
    );

    child.override_pin_value("pin-child", json!("child"));
    assert!(
        context
            .context_pin_overrides
            .as_ref()
            .is_some_and(|map| !map.contains_key("pin-child")),
        "a child's write must not be visible to the parent before the merge"
    );

    child.end_trace();
    context.push_sub_context(&mut child);

    let merged = context.context_pin_overrides.as_ref().unwrap();
    assert_eq!(
        merged.get("pin-child").map(|v| v.as_ref()),
        Some(&json!("child"))
    );
    assert_eq!(
        merged.get("pin-parent").map(|v| v.as_ref()),
        Some(&json!("parent"))
    );
}

#[tokio::test]
async fn siblings_cannot_see_each_others_overrides() {
    let (mut context, _channel) = context_with_callback("override-siblings", None).await;
    context.override_pin_value("shared", json!("base"));

    let node = context.node.clone();
    let mut first = context.create_sub_context(&node).await;
    let mut second = context.create_sub_context(&node).await;

    first.override_pin_value("shared", json!("first"));
    second.override_pin_value("shared", json!("second"));

    let first_value = first
        .context_pin_overrides
        .as_ref()
        .and_then(|map| map.get("shared"))
        .cloned();
    let second_value = second
        .context_pin_overrides
        .as_ref()
        .and_then(|map| map.get("shared"))
        .cloned();

    assert_eq!(first_value.as_deref(), Some(&json!("first")));
    assert_eq!(
        second_value.as_deref(),
        Some(&json!("second")),
        "one branch's override must never overwrite a sibling's"
    );
    assert_eq!(
        context
            .context_pin_overrides
            .as_ref()
            .and_then(|map| map.get("shared"))
            .map(|value| value.as_ref()),
        Some(&json!("base")),
        "the parent keeps its own value while branches are in flight"
    );
}

#[tokio::test]
async fn overriding_a_key_replaces_it_rather_than_mutating_the_shared_value() {
    let (mut context, _channel) = context_with_callback("override-replace", None).await;
    context.override_pin_value("pin", json!({ "n": 1 }));

    let node = context.node.clone();
    let mut child = context.create_sub_context(&node).await;
    // The child holds the same allocation as the parent until it writes.
    child.override_pin_value("pin", json!({ "n": 2 }));

    assert_eq!(
        context
            .context_pin_overrides
            .as_ref()
            .and_then(|map| map.get("pin"))
            .map(|value| value.as_ref()),
        Some(&json!({ "n": 1 })),
        "writing through a shared entry must not be visible to the other holder"
    );

    child.end_trace();
    context.push_sub_context(&mut child);

    assert_eq!(
        context
            .context_pin_overrides
            .as_ref()
            .and_then(|map| map.get("pin"))
            .map(|value| value.as_ref()),
        Some(&json!({ "n": 2 }))
    );
}
