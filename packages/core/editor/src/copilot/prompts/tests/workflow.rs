use super::*;

#[test]
fn board_organization_guidance_covers_module_blocks() {
    // Every board-capable prompt embeds this constant; the module contract must not silently
    // drop out of it. "Module" alone is ambiguous here — typed Flow IR uses the same word for
    // its function/event units — so the section is keyed by its namespace-file framing.
    for marker in [
        "MODULE BLOCKS (NAMESPACE FILES)",
        "module name { ... }",
        "ABSOLUTE path from the root",
        "board-global",
        "written text is authoritative",
        "OUT of its module",
    ] {
        assert!(
            BOARD_ORGANIZATION_GUIDANCE.contains(marker),
            "module-block guidance lost `{marker}`"
        );
    }
}

#[test]
fn board_prompts_expose_function_cache_syntax_and_runtime_semantics() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_system_prompt(),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
    ];

    for prompt in prompts {
        assert!(prompt.contains("## FUNCTION RESULT CACHING"));
        assert!(
            prompt.contains(r#"@cache({ namespace: "pricing", ttlSeconds: 3600, scope: "user" })"#)
        );
        assert!(prompt.contains("A bare `@cache` enables the defaults"));
        assert!(prompt.contains("`\"global\"` namespace"));
        assert!(prompt.contains("300-second lifetime"));
        assert!(prompt.contains("`ttlSeconds: 0` explicitly for a permanent entry"));
        assert!(prompt.contains("`ttl_seconds: null`"));
        assert!(prompt.contains("permanent cache"));
        assert!(prompt.contains("`scope` is"));
        assert!(prompt.contains("exactly `\"app\"` or `\"user\"`"));
        assert!(prompt.contains("ENTIRE function body is skipped"));
        assert!(prompt.contains("including every side effect"));
        assert!(prompt.contains("Preserve an existing `@cache` decorator"));
    }
}

/// The renderer now emits `ns::alias`, method calls, `use` lines and template literals; a
/// prompt that still teaches only the flat spelling makes the model fight the board text it
/// is shown.
#[test]
fn board_prompts_teach_namespaces_methods_and_sugar() {
    for prompt in [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
    ] {
        assert!(prompt.contains("## FLOWSCRIPT NAMES AND SUGAR"));
        assert!(prompt.contains("`use ns::*` at the TOP of the file"));
        assert!(prompt.contains("A declaration with a `this:` parameter is also a METHOD"));
        assert!(prompt.contains("The legacy camelCase name"));
        assert!(prompt.contains("Template literals lower to `string::format`"));
        assert!(prompt.contains("for (const [index, item] of items)"));
        assert!(prompt.contains("Destructure MULTI-output nodes by pin name"));
        assert!(prompt.contains("A single-output call is the value"));
        assert!(prompt.contains("never rename an argument"));
        assert!(!prompt.contains("There is no unary minus"));
        assert!(!prompt.contains("Do not invent aliases"));
    }
    for example in FLOWSCRIPT_FEW_SHOT_EXAMPLES
        .split("```flowscript-verified\n")
        .skip(1)
    {
        let body = example
            .split_once("\n```")
            .map(|(body, _)| body)
            .unwrap_or("");
        assert!(
            body.contains("::") || body.contains("use "),
            "verified example must show the namespaced spelling:\n{body}"
        );
    }
}

#[test]
fn board_prompts_preserve_failed_full_scope_drafts() {
    let prompt = board_sdk_flowscript_system_prompt("", 0);
    assert!(prompt.contains("requested behavior as an invariant"));
    assert!(prompt.contains("last submitted draft plus its"));
    assert!(prompt.contains("diagnostics"));
    assert!(prompt.contains("`RECOVERED CANDIDATE` / `retained_candidate`"));
    assert!(prompt.contains("active FlowScript workspace"));
    assert!(prompt.contains("platform-orchestration regression"));
    assert!(prompt.contains("continue the retained production candidate"));
    assert!(prompt.contains("literal `function` keyword"));
    assert!(prompt.contains("Catalog/type validity proves graph shape"));
    assert!(prompt.contains("must declare a named return pin"));
    assert!(prompt.contains("Never call shell/file/Read tools"));
    let rig_prompt = board_system_prompt("{}", "", 0, false, false);
    assert!(rig_prompt.contains("position-only MoveNode"));
    assert!(!rig_prompt.contains("Simple Event command last"));
}

#[test]
fn board_prompts_explain_repeated_string_format_placeholders() {
    assert!(NUMBERS_CONVERSIONS_GUIDANCE.contains("Repeating `{name}` reuses that same pin"));
    assert!(NUMBERS_CONVERSIONS_GUIDANCE.contains("typed IR: occurrence `0`"));

    for prompt in [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ] {
        assert!(prompt.contains("Repeating `{name}` reuses that same pin"));
        assert!(prompt.contains("typed IR: occurrence `0`"));
    }
}

#[test]
fn board_prompts_explain_configuration_derived_pins() {
    // Both halves matter: the model has to know these pins exist at all (nothing in
    // `get_declarations` lists them), and that the driving config has to be in the same
    // call — a value supplied later has no pin to land on.
    for prompt in [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ] {
        assert!(prompt.contains("PINS THAT ONLY EXIST AFTER CONFIGURATION"));
        assert!(prompt.contains("MUST be in the SAME call"));
        // SQL parameters: the naming rule and the injection prohibition.
        assert!(prompt.contains("`param<Name>`"));
        assert!(prompt.contains("never concatenated into the SQL"));
        assert!(prompt.contains("Placeholders stand for VALUES ONLY"));
        assert!(prompt.contains("array_has($ids, id)"));
        // Widget bindings: every prefix, and where the real names come from.
        for prefix in [
            "dynPath<Field>",
            "dynProp<Id>",
            "dynCust<Id>",
            "dynIn<Key>",
            "dynArg<Key>",
        ] {
            assert!(prompt.contains(prefix), "missing {prefix}");
        }
        assert!(prompt.contains("operation `widget` lists the exact pin names"));
    }
}

#[test]
fn board_prompts_cover_numbers_conversions_and_draft_continuation() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
        board_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("## NUMBERS & CONVERSIONS"));
        assert!(prompt.contains("NEVER invoke an LLM/agent node for arithmetic"));
        assert!(prompt.contains("no `json::toInt`/`json::toFloat` catalog node"));
        assert!(prompt.contains("No no-op identity calls"));
    }

    for prompt in [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
    ] {
        assert!(prompt.contains("SAME draft_id and exact\n   expected_revision"));
        assert!(prompt.contains("never start a new draft id"));
        assert!(prompt.contains("each declared return pin needs a matching return value"));
        assert!(prompt.contains("An event-level `return` accepts exactly\n  one value"));
        assert!(prompt.contains("Never reassign a `const` binding inside a branch arm"));
        assert!(prompt.contains("df::createSession({ sessionName: \"default\" })"));
        assert!(!prompt.contains("collectStatistics: true"));
        assert!(prompt.contains("one field on an existing struct"));
        assert!(prompt.contains(r#"row.status = "done""#));
        assert!(prompt.contains(r#"row = row.set({ field: "status", value: "done" })"#));
        assert!(prompt.contains("both preserve its other fields"));
    }
}

#[test]
fn board_prompts_make_flowscript_the_only_model_facing_workflow_surface() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
    ];

    for prompt in prompts {
        assert!(prompt.contains("## PRIMARY SURFACE: FlowScript"));
        assert!(
                prompt.contains(
                    "For a new or empty board, start a complete source document from the requested behavior."
                )
            );
        assert!(prompt.contains("live inline preview"));
        assert!(prompt.contains("**Build or modify FlowScript**"));
        for source_tool in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            assert!(
                prompt.contains(source_tool),
                "model-facing prompt omitted source lifecycle tool: {source_tool}"
            );
        }
        assert!(!prompt.contains("edit_flowscript"));
        assert!(prompt.contains("position-only MoveNode"));
        assert!(prompt.contains("CreateComment"));
        assert!(prompt.contains("DeleteComment"));
        assert!(prompt.contains("creation/removal"));
        assert!(!prompt.contains("## Commands"));
        assert!(!prompt.contains("## emit_commands FORMAT"));
        assert!(!prompt.contains("AddPlaceholder(name"));
        assert!(!prompt.contains("\"command_type\": \"AddNode\""));

        for legacy_typed_surface in [
            "TYPED FLOW IR",
            "plan_flow_ir",
            "begin_flow_ir_draft",
            "update_flow_ir_draft",
            "upsert_flow_ir_module",
            "validate_flow_ir_draft",
            "commit_flow_ir_draft",
            "flow-ir-verified",
        ] {
            assert!(
                !prompt.contains(legacy_typed_surface),
                "model-facing prompt still exposes legacy surface: {legacy_typed_surface}"
            );
        }
    }
}

#[test]
fn board_prompts_bound_discovery_and_retain_a_full_shape_draft_early() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
    ];

    for prompt in prompts {
        assert!(prompt.contains("ONE bounded, focused `get_declarations`"));
        assert!(prompt.contains("highest-leverage catalog signatures"));
        assert!(prompt.contains("After the plan is accepted"));
        assert!(prompt.contains("edits: [{old_text, new_text}]"));
        assert!(prompt.contains("including every unchanged anchor"));
        assert!(prompt.contains("Omit `source` when using `edits`"));
        assert!(!prompt.contains("Always call `write_flowscript` with the complete source"));
        assert!(prompt.contains("Both forms require executable changes"));
        assert!(prompt.contains("whole\n   request for a single-segment plan"));
        assert!(prompt.contains("omitted or unmatched declaration searches"));
        assert!(prompt.contains("compiler diagnostics"));
        assert!(prompt.contains("at most six total ancillary inspection calls"));
        assert!(!prompt.contains("containing every catalog\n   signature"));
        assert!(!prompt.contains("with every needed search\n   batched"));
    }
}

/// Every board builder concatenates the guidance list independently, so a new block silently
/// reaches only the builder it was pasted into. This is the guard for that.
#[test]
fn board_prompts_require_a_scope_plan_before_the_first_source_write() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_system_prompt(),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
    ];

    for prompt in prompts {
        assert!(prompt.contains("## SCOPE SEGMENTATION"));
        assert!(prompt.contains("`plan_board_scope`"));
        assert!(prompt.contains("A SEGMENT IS NOT A STUB"));
        assert!(prompt.contains("Segmentation is HOW the request is built"));
        assert!(prompt.contains("Boards of one app CANNOT call each other"));
    }
}

/// A FlowPath carries no file attributes, so a board prompt without this block produces
/// `file.filename` reads that resolve to null instead of the catalog accessors.
#[test]
fn board_prompts_teach_flow_path_accessors() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_system_prompt(),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
    ];

    for prompt in prompts {
        assert!(prompt.contains("## FILES ARE FlowPath HANDLES, NOT FIELD BAGS"));
        assert!(prompt.contains("`path`, `storeRef`, `cacheStoreRef`"));
        assert!(prompt.contains("NEVER read a file attribute with dot access or `struct::get`"));
        for accessor in [
            "filename({ path: file })",
            "extension({ path: file })",
            "rawPath({ path: file })",
            "parent({ path: file })",
        ] {
            assert!(prompt.contains(accessor), "missing {accessor}");
        }
    }
}

/// Every board prompt must carry the escape hatch, or one impossible step still ends a run with
/// an empty board — the outcome this guidance exists to prevent.
#[test]
fn board_prompts_offer_a_stub_instead_of_abandoning_the_build() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_system_prompt(),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
    ];

    for prompt in prompts {
        assert!(prompt.contains("## NEVER GIVE UP: STUB THE UNBUILDABLE UNIT INSTEAD"));
        assert!(prompt.contains("You do\nnot have the option of abandoning the build"));
        assert!(prompt.contains("Declare the function with its REAL interface"));
        assert!(prompt.contains(UNIMPLEMENTED_STUB_MARKER));
        assert!(prompt.contains("This is a LAST RESORT for one unit, never a strategy"));
    }
}

/// The host scans committed log messages for this literal to build the manual-step list the
/// orchestrator relays. If the guidance stops telling the model to emit it, every gap a build
/// hands back silently disappears from the user's report.
#[test]
fn stub_marker_stays_in_sync_with_the_guidance_that_produces_it() {
    assert!(UNBUILDABLE_UNIT_GUIDANCE.contains(UNIMPLEMENTED_STUB_MARKER));
    assert!(UNBUILDABLE_UNIT_GUIDANCE.contains("the exact literal `NOT IMPLEMENTED:`"));
}

/// A long build must know that time is bought with evidence, and that a refusal means stop
/// rather than rewrite — otherwise the extra hours just buy a longer loop.
#[test]
fn board_prompts_explain_that_wall_clock_is_earned_by_progress() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_system_prompt(),
        board_sdk_flowscript_system_prompt("", 0),
        general_system_prompt(),
    ];

    for prompt in prompts {
        assert!(prompt.contains("### TIME IS EARNED, NOT GIVEN"));
        assert!(prompt.contains("`extend_time_budget`"));
        assert!(prompt.contains("TIME_EXTENSION_NO_PROGRESS"));
        assert!(prompt.contains("That is a signal to STOP, not to rewrite"));
        assert!(prompt.contains("Extra time never relaxes the repair budget"));
    }
}

/// A one-segment plan must stay the documented default, or every trivial edit pays for a
/// decomposition it does not need.
#[test]
fn scope_segmentation_keeps_single_segment_as_the_common_case() {
    assert!(
        SCOPE_SEGMENTATION_GUIDANCE
            .contains("An ordinary edit is a ONE-segment plan with `strategy: \"single\"`")
    );
    assert!(SCOPE_SEGMENTATION_GUIDANCE.contains("do not invent segments"));
    assert!(SCOPE_SEGMENTATION_GUIDANCE.contains("Split only when"));
}

/// The no-board-to-board rule bounds connected logic only. Pages talk through app data and
/// element refs, so they are the case `multi_board` exists for; a prompt that reads as a blanket
/// warning pushes every page of a multi-page app onto one unsplittable board.
#[test]
fn scope_segmentation_makes_per_page_boards_the_ordinary_multi_board_case() {
    assert!(SCOPE_SEGMENTATION_GUIDANCE.contains("PAGES are the standard multi-board case"));
    assert!(SCOPE_SEGMENTATION_GUIDANCE.contains("ONE BOARD PER PAGE"));
    assert!(SCOPE_SEGMENTATION_GUIDANCE.contains("one board per page is the ordinary case"));
    assert!(
        SCOPE_SEGMENTATION_GUIDANCE
            .contains("Never use `\"multi_board\"` to split one connected program")
    );
    assert!(SCOPE_SEGMENTATION_GUIDANCE.contains("Boards of one app CANNOT call each other"));
}

#[test]
fn event_entry_guidance_requires_named_purpose_events() {
    let prompts = [
        board_system_prompt("{}", "", 0, false, false),
        board_sdk_flowscript_system_prompt("", 0),
        board_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("one NAMED event per purpose"));
        assert!(prompt.contains("eventsSimple dashboardLoad() { ... }"));
        assert!(prompt.contains("checkTargetsCron() { ... }"));
        assert!(prompt.contains("\"Simple Event\"/\"Generic Event\" is a defect"));
        assert!(prompt.contains("Distinct purposes get distinct entries"));
    }
}
