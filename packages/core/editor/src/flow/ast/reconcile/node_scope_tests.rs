use super::*;

fn directory_catalog() -> Vec<NodeMetadata> {
    let mut catalog = vec![
        catalog_meta(
            "events_simple",
            "Simple Event",
            vec![],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        ),
        catalog_meta(
            "consume",
            "Consume",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("path", "Struct", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        ),
    ];
    for (node_type, alias) in [
        ("path_from_cache_dir", "cacheDir"),
        ("path_from_storage_dir", "storageDir"),
        ("path_from_user_dir", "userDir"),
    ] {
        let mut scope = pin_meta("node_scope", "Boolean", PinType::Input);
        scope.default_value = Some("false".into());
        let mut meta = catalog_meta(
            node_type,
            alias,
            vec![scope],
            vec![pin_meta("path", "Struct", PinType::Output)],
        );
        meta.namespace = Some("files".into());
        meta.alias = Some(alias.into());
        catalog.push(meta);
    }
    catalog
}

#[test]
fn repeated_node_scope_warns_for_each_directory_in_functions_and_events() {
    for helper in ["cacheDir", "storageDir", "userDir"] {
        for (header, body) in [
            (
                "function paths(): (a: Struct, b: Struct)",
                format!(
                    "return files::{helper}({{ nodeScope: true }}), files::{helper}({{ nodeScope: true }})"
                ),
            ),
            (
                "eventsSimple()",
                format!(
                    "consume({{ path: files::{helper}({{ nodeScope: true }}) }})\nconsume({{ path: files::{helper}({{ nodeScope: true }}) }})"
                ),
            ),
        ] {
            let result = reconcile_text_with_catalog(
                &empty_board(),
                &format!("{header} {{\n{body}\n}}"),
                &directory_catalog(),
            );
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            assert!(!result.commands.is_empty());
            assert_eq!(result.warnings.len(), 1, "{header}: {:?}", result.warnings);
            assert!(result.warnings[0].contains(&format!("files::{helper}")));
            assert!(result.warnings[0].contains("shared path variable"));
            assert!(result.warnings[0].contains("nodeScope: false"));
        }
    }
}

#[test]
fn repeated_node_scope_resolves_aliases_legacy_names_and_positional_inputs() {
    for (imports, first, second) in [
        (
            "use files as fs",
            "fs::cacheDir({ nodeScope: true })",
            "files::cacheDir({ node_scope: true })",
        ),
        (
            "use files::{cacheDir as cache}",
            "cache(true)",
            "files::cacheDir({ nodeScope: true })",
        ),
        (
            "use files::*",
            "cacheDir({ nodeScope: true })",
            "pathFromCacheDir(true)",
        ),
    ] {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            &format!(
                "{imports}\nfunction paths(): (a: Struct, b: Struct) {{\nreturn {first}, {second}\n}}"
            ),
            &directory_catalog(),
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.warnings.len(), 1, "{imports}: {:?}", result.warnings);
    }
}

#[test]
fn node_scope_does_not_warn_for_shared_paths_or_false_omitted_and_dynamic_scope() {
    for body in [
        "const shared = files::cacheDir({ nodeScope: true })\nreturn shared, shared",
        "return files::cacheDir({ nodeScope: true }), files::cacheDir({ nodeScope: false })",
        "return files::cacheDir({ nodeScope: true }), files::cacheDir()",
        "return files::cacheDir({ nodeScope: true }), files::cacheDir({ nodeScope: enabled })",
        "return files::cacheDir({ nodeScope: true }), files::storageDir({ nodeScope: true })",
    ] {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            &format!("function paths(enabled: bool): (a: Struct, b: Struct) {{\n{body}\n}}"),
            &directory_catalog(),
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.warnings.is_empty(), "{body}: {:?}", result.warnings);
    }
}

#[test]
fn node_scope_counts_are_separate_for_functions_events_and_modules() {
    let source = r#"
function first(): (path: Struct) { return files::cacheDir({ nodeScope: true }) }
function second(): (path: Struct) { return files::cacheDir({ nodeScope: true }) }
eventsSimple() { consume({ path: files::cacheDir({ nodeScope: true }) }) }
eventsSimple() { consume({ path: files::cacheDir({ nodeScope: true }) }) }
module nested {
    function nestedPath(): (path: Struct) { return files::cacheDir({ nodeScope: true }) }
}
"#;
    let result = reconcile_text_with_catalog(&empty_board(), source, &directory_catalog());
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
}

#[test]
fn node_scope_warns_across_nested_blocks_but_handlers_are_separate() {
    let ast = flow_like_ast::parse(
        r#"
function paths() {
    const first = files::cacheDir({ nodeScope: true })
    if (true) {
        @collapse("Paths") {
            for (const value of [1]) {
                const second = files::cacheDir({ nodeScope: true })
            }
        }
    }
    eventsSimple() {
        const separate = files::cacheDir({ nodeScope: true })
    }
}
"#,
    )
    .unwrap();
    let board = empty_board();
    let catalog = directory_catalog();
    let mut planner = StructuralPlanner::new(&board, &catalog, None);
    planner.warn_repeated_node_scoped_dirs(&ast.functions[0].body, "function `paths`");
    assert_eq!(planner.result.warnings.len(), 1);

    let ast = flow_like_ast::parse(
        r#"
function paths() {
    const first = files::cacheDir({ nodeScope: true })
    eventsSimple() { const separate = files::cacheDir({ nodeScope: true }) }
}
"#,
    )
    .unwrap();
    planner.result.warnings.clear();
    planner.warn_repeated_node_scoped_dirs(&ast.functions[0].body, "function `paths`");
    assert!(planner.result.warnings.is_empty());
}

#[test]
fn node_scope_ignores_unrelated_catalog_functions_and_repeated_node_identity() {
    let mut catalog = directory_catalog();
    let mut unrelated = catalog.last().unwrap().clone();
    unrelated.name = "unrelated_cache".into();
    unrelated.namespace = Some("other".into());
    unrelated.alias = Some("cacheDir".into());
    catalog.push(unrelated);
    let result = reconcile_text_with_catalog(
        &empty_board(),
        "function paths(): (a: Struct, b: Struct) { return other::cacheDir(true), other::cacheDir(true) }",
        &catalog,
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(result.warnings.is_empty());

    let mut ast = flow_like_ast::parse(
        "function paths(): (a: Struct, b: Struct) { return files::cacheDir(true), files::cacheDir(true) }",
    ).unwrap();
    let Stmt::Return { values, .. } = &mut ast.functions[0].body.stmts[0] else {
        panic!("return")
    };
    for value in values {
        let Expr::Call(call) = value else {
            panic!("call")
        };
        call.anchor = Some("shared-node".into());
    }
    let board = empty_board();
    let mut planner = StructuralPlanner::new(&board, &catalog, None);
    planner.warn_repeated_node_scoped_dirs(&ast.functions[0].body, "function `paths`");
    assert!(planner.result.warnings.is_empty());
}
