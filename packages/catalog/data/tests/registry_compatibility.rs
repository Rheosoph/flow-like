//! Keep every pre-split public node path and its catalog position available.

use flow_like_catalog_data::{self as catalog, NodeLogic};
use flow_like_runtime::flow::{pin::PinType, variable::VariableType};

macro_rules! assert_legacy_registry {
    (
        public: [$($node:path),* $(,)?],
        private: [$($private_name:literal),* $(,)?] $(,)?
    ) => {
        #[test]
        #[allow(clippy::default_constructed_unit_structs)]
        fn facade_preserves_public_nodes_and_registration_order() {
            let mut expected = vec![$(<$node>::default().get_node().name),*];
            expected.extend([$($private_name.to_owned()),*]);
            for (entry_point, nodes) in [
                ("collect_nodes", catalog::collect_nodes()),
                ("get_catalog", catalog::get_catalog()),
            ] {
                // New nodes may be added between legacy entries. Preserve every old
                // entry and their relative order across the split catalog facade.
                let actual: Vec<_> = nodes.iter().map(|node| node.get_node().name)
                    .filter(|name| expected.contains(name)).collect();
                assert_eq!(actual, expected, "{entry_point} changed the data catalog");
            }
        }
    };
}

include!("fixtures/legacy_node_paths.rs");

#[test]
fn chat_controls_are_registered_once_with_chat_aliases() {
    let nodes = catalog::collect_nodes();
    for (name, alias) in [
        ("events_chat_set_main_widget", "setMainWidget"),
        ("events_chat_clear_main_widget", "clearMainWidget"),
        ("events_chat_react", "react"),
    ] {
        let matches: Vec<_> = nodes
            .iter()
            .map(|logic| logic.get_node())
            .filter(|node| node.name == name)
            .collect();
        assert_eq!(matches.len(), 1, "Expected one registration for {name}");
        let node = &matches[0];
        assert_eq!(node.flowscript_namespace(), "chat");
        assert_eq!(node.flowscript_alias(), alias);
        for pin_name in ["exec_in", "exec_out"] {
            assert_eq!(
                node.get_pin_by_name(pin_name).unwrap().data_type,
                VariableType::Execution
            );
        }
        if alias == "setMainWidget" {
            let widget = node.get_pin_by_name("element_ref").unwrap();
            assert_eq!(widget.data_type, VariableType::Struct);
            assert!(widget.schema.is_some());
            let display = node.get_pin_by_name("display").unwrap();
            assert_eq!(
                display
                    .options
                    .as_ref()
                    .unwrap()
                    .valid_values
                    .as_ref()
                    .unwrap(),
                &vec![
                    "open".to_string(),
                    "collapsed".to_string(),
                    "maximized".to_string()
                ]
            );
        }
    }
}

#[test]
fn database_reference_nodes_are_registered_once() {
    let nodes = catalog::collect_nodes();
    for name in [
        "database_reference",
        "database_versions",
        "database_branches",
        "database_tags",
        "database_checkout",
        "database_snapshot",
        "database_create_branch",
        "database_delete_branch",
        "database_create_tag",
        "database_update_tag",
        "database_delete_tag",
        "database_restore",
        "database_cleanup_versions",
        "database_compare",
        "database_clone",
    ] {
        let matches: Vec<_> = nodes
            .iter()
            .map(|logic| logic.get_node())
            .filter(|node| node.name == name)
            .collect();
        assert_eq!(matches.len(), 1, "Expected one registration for {name}");
        assert!(matches[0].get_pin_by_name("reference").is_some());
        assert!(matches[0].get_pin_by_name("database").is_some());
    }
}

#[test]
fn path_type_nodes_are_registered_once_with_boolean_outputs() {
    for (entry_point, nodes) in [
        ("collect_nodes", catalog::collect_nodes()),
        ("get_catalog", catalog::get_catalog()),
    ] {
        for (name, output_name, alias) in [
            ("path_is_file", "is_file", "isFile"),
            ("path_is_folder", "is_folder", "isFolder"),
        ] {
            let matches: Vec<_> = nodes
                .iter()
                .map(|logic| logic.get_node())
                .filter(|node| node.name == name)
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "Expected one registration for {name} in {entry_point}"
            );
            let node = &matches[0];
            assert_eq!(node.flowscript_namespace(), "files");
            assert_eq!(node.flowscript_alias(), alias);
            assert_eq!(node.flowscript_receiver().as_deref(), Some("path"));

            let path = node.get_pin_by_name("path").unwrap();
            assert_eq!(path.pin_type, PinType::Input);
            assert_eq!(path.data_type, VariableType::Struct);
            assert!(path.schema.is_some());
            assert_eq!(
                path.options
                    .as_ref()
                    .and_then(|options| options.enforce_schema),
                Some(true)
            );

            let output = node.get_pin_by_name(output_name).unwrap();
            assert_eq!(output.pin_type, PinType::Output);
            assert_eq!(output.data_type, VariableType::Boolean);

            for (pin_name, pin_type) in [("exec_in", PinType::Input), ("exec_out", PinType::Output)]
            {
                let pin = node.get_pin_by_name(pin_name).unwrap();
                assert_eq!(pin.pin_type, pin_type);
                assert_eq!(pin.data_type, VariableType::Execution);
            }
        }
    }
}
