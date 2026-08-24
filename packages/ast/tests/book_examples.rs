//! Keeps published FlowBook examples parseable and in canonical FlowScript form.

use flow_like_ast::{RenderOptions, parse, render};

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
    let source =
        include_str!("../../../apps/book/examples/types-and-interfaces/types.flow");
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
