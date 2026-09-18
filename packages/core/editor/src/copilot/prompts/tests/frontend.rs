use super::*;

#[test]
fn frontend_prompts_defer_new_page_persistence_until_host_apply() {
    let docs = crate::a2ui::copilot::get_full_documentation();
    for prompt in [
        frontend_system_prompt("{}", &docs),
        frontend_sdk_system_prompt(),
    ] {
        assert!(prompt.contains("host creates or updates the target page only after you return"));
        assert!(prompt.contains("Do not inspect a newly emitted page"));
        assert!(prompt.contains("host can apply it and report authoritative persistence evidence"));
    }
}

#[test]
fn frontend_prompts_demand_design_reflection_and_true_styling_channels() {
    let docs = crate::a2ui::copilot::get_full_documentation();
    let prompts = [
        frontend_system_prompt("{}", &docs),
        frontend_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("## DESIGN CONTRACT (run this before every emit_ui)"));
        assert!(prompt.contains("## Design Reflection (BEFORE emitting)"));
        assert!(prompt.contains("compiled at runtime against this theme"));
        assert!(!prompt.to_lowercase().contains("no runtime tailwind engine"));
        assert!(prompt.contains("responsiveOverrides"));
        assert!(prompt.contains("canvasSettings.customCss"));
        assert!(prompt.contains("NEVER `:root`"));
        assert!(prompt.contains("## Choosing the Right Component"));
        assert!(prompt.contains("`voiceInput`"));
        assert!(prompt.contains("never a button + fileInput imitation"));
        assert!(prompt.contains("usable at 360px wide"));
    }
}

/// The anti-convergence mechanism: a declared, stamped design tuple with a checkable distance
/// rule, a concrete blocklist of observed defaults, and a pass/fail gate. Open-ended "be
/// creative" instructions provably collapse back to the default attractor, so each of these
/// pieces is load-bearing rather than decorative.
#[test]
fn frontend_prompts_force_a_declared_and_stamped_design_direction() {
    let docs = crate::a2ui::copilot::get_full_documentation();
    let prompts = [
        frontend_system_prompt("{}", &docs),
        frontend_sdk_system_prompt(),
    ];
    for prompt in prompts {
        assert!(prompt.contains("You converge."));
        assert!(prompt.contains("Subject-independence"));
        for axis in ["macro:", "surface:", "type:", "density:"] {
            assert!(prompt.contains(axis), "design taxonomy must define {axis}");
        }
        assert!(prompt.contains("differ from every prior stamp on at least TWO"));
        assert!(prompt.contains("/* fp-design: macro="));
        assert!(prompt.contains("## BANNED DEFAULTS"));
        assert!(prompt.contains("## TREATMENT CALIBRATION"));
        assert!(prompt.contains("## PRE-EMIT GATE"));
        assert!(prompt.contains("## TOKEN LOCK"));
        assert!(prompt.contains("INVENTED DATA"));
        assert!(prompt.contains("font-sans/font-serif/font-mono"));
    }
}
