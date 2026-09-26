use super::*;

#[test]
fn database_setup_cannot_block_the_first_board_mutation() {
    let prompt = board_sdk_flowscript_system_prompt("", 0);
    assert!(prompt.contains("database setup is\nnever a prerequisite"));
    assert!(prompt.contains("call `plan_board_scope` exactly once"));
    assert!(prompt.contains("submit its active segment through `write_flowscript`"));
    assert!(prompt.contains("One such result proves the capability mismatch"));
    assert!(
        prompt.contains(
            "Record any remaining requested schemas as pending and finish/apply the board"
        )
    );
}

#[test]
fn ontology_query_prompt_requires_one_tool_free_read_only_json_proposal() {
    let prompt = ontology_query_system_prompt();
    assert!(prompt.contains("You have no tools"));
    assert!(prompt.contains("read-only Cypher or SQL"));
    assert!(prompt.contains("Return exactly one JSON object"));
    assert!(prompt.contains("\"presentation\":\"graph|table\""));
    assert!(prompt.contains("Treat schema names"));
    assert!(prompt.contains("`$name` syntax"));
}

#[test]
fn database_guidance_teaches_lazy_first_write_table_bootstrap() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("explicit_schema_create_not_deployed"));
        assert!(prompt.contains("HTTP 405 on a local runtime"));
        assert!(prompt.contains("The portable bootstrap is LAZY"));
        assert!(prompt.contains("upsert one COMPLETE first row"));
        assert!(prompt.contains("zero-filled vector for vector columns"));
        assert!(prompt.contains("lazy first-write bootstrap by default"));
    }
}

#[test]
fn failed_database_or_index_setup_never_abandons_the_build() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(
            prompt.contains("### A DATABASE OR INDEX YOU COULD NOT SET UP NEVER STOPS THE BUILD")
        );
        assert!(prompt.contains("an index that cannot be built"));
        assert!(prompt.contains("BUILD THE WORKFLOW ANYWAY"));
        assert!(prompt.contains("not an unbuildable unit"));
        assert!(prompt.contains("exact vector width of the embedding model"));
        assert!(
            prompt.contains(
                "Build indices IN THE FLOW with `db::buildIndex`, AFTER that first write"
            )
        );
    }
    assert!(
        UNBUILDABLE_UNIT_GUIDANCE
            .contains("could not be created out of band is NOT an unbuildable unit")
    );
}

#[test]
fn data_studio_guidance_normalizes_human_table_labels() {
    let prompt = data_studio_system_prompt("");
    assert!(prompt.contains("normalizes it to stable snake_case"));
    assert!(prompt.contains("authoritative `table_name`"));
    assert!(prompt.contains("continue the requested build"));
    assert!(prompt.contains("Do not stop to search for a separate"));
}

#[test]
fn data_studio_guidance_imports_forwarded_geojson_files() {
    let prompt = data_studio_system_prompt("");
    assert!(prompt.contains("import_geojson"));
    assert!(prompt.contains("FILES FORWARDED FOR IMPORT"));
    assert!(prompt.contains("never retype coordinates"));
    assert!(prompt.contains("never present them as metres"));
    assert!(prompt.contains("sentinel strings such as `\"unknown\"` stays `string`"));
}

#[test]
fn table_drops_are_data_studio_only_and_never_a_reset() {
    let data_prompt = data_studio_system_prompt("");
    assert!(data_prompt.contains("delete_table"));
    assert!(data_prompt.contains("IRREVERSIBLE"));
    assert!(data_prompt.contains("confirm_table_name"));
    assert!(data_prompt.contains("ontologies_pruned"));
    assert!(data_prompt.contains("saved_queries_referencing"));

    for prompt in [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_system_prompt(),
        board_sdk_flowscript_system_prompt("", 0),
    ] {
        assert!(prompt.contains("delete_table"));
        assert!(!prompt.contains("confirm_table_name"));
    }
}

#[test]
fn temporal_contract_pairs_data_studio_utc_timestamps_with_board_dates() {
    let data_prompt = data_studio_system_prompt("");
    assert!(data_prompt.contains("`\"timestamp:ms:UTC\"`"));
    assert!(data_prompt.contains("FlowLike board `Date`"));
    assert!(data_prompt.contains("Never create such a column as `string`, `date32`, or a"));
    assert!(data_prompt.contains("existing Utf8/LargeUtf8 column"));

    for prompt in [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ] {
        assert!(prompt.contains("### DATE/TIME TYPE CONTRACT"));
        assert!(prompt.contains("use `Date` for the field"));
        assert!(prompt.contains("`datetime::now`"));
        assert!(prompt.contains("`datetime::parse`"));
        assert!(prompt.contains("`type: \"timestamp:ms:UTC\"`"));
        assert!(prompt.contains("Utf8/LargeUtf8"));
        assert!(prompt.contains("`to_timestamp(column)`"));
        assert!(prompt.contains("only the legacy raw column is\n`string`"));
        assert!(prompt.contains("sort/filter it directly"));
    }
}

#[test]
fn board_guidance_requires_real_uploaded_document_extraction() {
    assert!(
        DATABASE_WORKFLOW_GUIDANCE.contains("a file picker or chat attachment yields a `FlowPath`")
    );
    assert!(DATABASE_WORKFLOW_GUIDANCE.contains("`ai_processing_extract_document`"));
    assert!(DATABASE_WORKFLOW_GUIDANCE.contains("Never replace\n  extraction with a filename"));
}

#[test]
fn dashboard_updates_prefer_element_setters_over_data_update() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("## A2UI PAGES: UPDATING WHAT AN ELEMENT SHOWS"));
        assert!(prompt.contains("It writes to the ELEMENT with that element's\nsetter"));
        for setter in [
            "ui::setElementText",
            "ui::setElementValue",
            "ui::writeCsvToTable",
            "ui::pushCsvToChart",
            "ui::instantiateWidget",
            "ui::widgetUpdateInputs",
            "ui::pushChild",
        ] {
            assert!(prompt.contains(setter), "missing element setter: {setter}");
        }
        assert!(prompt.contains("`ui::dataUpdate`) is FORBIDDEN"));
        assert!(prompt.contains("FS_PROHIBITED_NODE"));
        assert!(!prompt.contains("`ui::dataUpdate` is a LAST RESORT"));
        assert!(!prompt.contains("`ui::dataUpdate`) is a LAST RESORT"));
        assert!(!prompt.contains("This is the ONLY node that updates the live UI"));
        assert!(!prompt.contains("visible now -> `ui::dataUpdate`"));
    }
}

#[test]
fn dashboard_guidance_makes_interaction_events_pull_their_inputs() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("### Interaction events PULL their own inputs"));
        assert!(prompt.contains("NEVER declare a Generic Event with payload parameters"));
        assert!(prompt.contains("ui::getElementValue({ elementRef }).value"));
        assert!(prompt.contains("ui::getFileInputFiles"));
        assert!(prompt.contains("addTarget() {"));
        assert!(prompt.contains("refreshTargetsTable()"));
    }
}
