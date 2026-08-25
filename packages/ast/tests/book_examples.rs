//! Keeps published FlowBook examples parseable and in canonical FlowScript form.

use flow_like_ast::{parse, render, RenderOptions};

#[test]
fn incident_triage_is_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/incident-triage/triage.flow");
    let ast = parse(source).expect("the Incident Triage book fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn document_anatomy_is_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/document-anatomy/anatomy.flow");
    let ast = parse(source).expect("the Chapter 6 document-anatomy fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn types_and_interfaces_are_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/types-and-interfaces/types.flow");
    let ast = parse(source).expect("the Chapter 7 types-and-interfaces fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn calling_the_catalog_is_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/calling-the-catalog/catalog.flow");
    let ast = parse(source).expect("the Chapter 8 catalog-calling fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn readable_sugar_is_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/readable-sugar/sugar.flow");
    let ast = parse(source).expect("the Chapter 9 readable-sugar fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn control_flow_is_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/control-flow/control.flow");
    let ast = parse(source).expect("the Chapter 10 control-flow fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn state_and_secrets_are_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/state-and-secrets/state.flow");
    let ast = parse(source).expect("the Chapter 11 state-and-secrets fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn functions_and_caching_are_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/functions-and-caching/functions.flow");
    let ast = parse(source).expect("the Chapter 12 functions-and-caching fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn events_and_interfaces_are_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/events-and-interfaces/events.flow");
    let ast = parse(source).expect("the Chapter 13 events-and-interfaces fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}

#[test]
fn board_ast_text_is_canonical_flowscript() {
    let source = include_str!("../../../apps/book/examples/board-ast-text/canonical.flow");
    let ast = parse(source).expect("the Chapter 14 Board-AST-text fixture should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        source,
        "the published fixture should already use canonical rendering"
    );
}
