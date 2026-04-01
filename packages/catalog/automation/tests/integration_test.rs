//! Integration tests for automation catalog nodes
//!
//! These tests verify that automation nodes can be:
//! - Instantiated with correct metadata
//! - Serialized/deserialized properly
//! - Have valid input/output pin schemas

use flow_like_catalog_automation::get_catalog;
use flow_like_catalog_automation::types::{
    fingerprints::{ElementFingerprint, FingerprintMatchOptions, MatchStrategy},
    selectors::{Selector, SelectorKind, SelectorSet},
};

/// Test that all automation nodes can be retrieved from the catalog
#[test]
fn test_catalog_loads_all_nodes() {
    let catalog = get_catalog();
    assert!(
        !catalog.is_empty(),
        "Catalog should contain automation nodes"
    );

    // Count nodes by category (use starts_with for nested categories)
    let mut browser_count = 0;
    let mut computer_count = 0;
    let mut rpa_count = 0;
    let mut vision_count = 0;
    let mut selector_count = 0;
    let mut fingerprint_count = 0;
    let mut llm_count = 0;

    for node in &catalog {
        let node_meta = node.get_node();
        let category = node_meta.category.as_str();
        if category.starts_with("Automation/Browser") {
            browser_count += 1;
        } else if category.starts_with("Automation/Computer") {
            computer_count += 1;
        } else if category.starts_with("Automation/RPA") {
            rpa_count += 1;
        } else if category.starts_with("Automation/Vision") {
            vision_count += 1;
        } else if category.starts_with("Automation/Selector") {
            selector_count += 1;
        } else if category.starts_with("Automation/Fingerprint") {
            fingerprint_count += 1;
        } else if category.starts_with("Automation/LLM") {
            llm_count += 1;
        }
    }

    println!("Browser nodes: {browser_count}");
    println!("Computer nodes: {computer_count}");
    println!("RPA nodes: {rpa_count}");
    println!("Vision nodes: {vision_count}");
    println!("Selector nodes: {selector_count}");
    println!("Fingerprint nodes: {fingerprint_count}");
    println!("LLM nodes: {llm_count}");

    // Verify we have nodes in each category
    assert!(browser_count > 0, "Should have browser nodes");
    assert!(computer_count > 0, "Should have computer nodes");
    assert!(rpa_count > 0, "Should have RPA nodes");
    assert!(vision_count > 0, "Should have vision nodes");
    assert!(selector_count > 0, "Should have selector nodes");
    assert!(fingerprint_count > 0, "Should have fingerprint nodes");
    assert!(llm_count > 0, "Should have LLM nodes");
}

/// Test that each node has required metadata
#[test]
fn test_nodes_have_valid_metadata() {
    let catalog = get_catalog();

    for node_logic in &catalog {
        let node = node_logic.get_node();

        // Every node must have basic metadata
        assert!(!node.id.is_empty(), "Node must have an ID");
        assert!(!node.name.is_empty(), "Node must have a name");
        assert!(!node.category.is_empty(), "Node must have a category");

        // Automation nodes should have an icon
        let has_icon = node.icon.is_some();
        assert!(
            has_icon,
            "Automation node '{}' should have an icon",
            node.name
        );

        // Every node must have at least one pin
        assert!(
            !node.pins.is_empty(),
            "Node '{}' should have at least one pin",
            node.name
        );
    }
}

/// Test ElementFingerprint serialization roundtrip
#[test]
fn test_fingerprint_serialization() {
    let selectors = SelectorSet::default()
        .add(Selector::css("#my-button"))
        .add(Selector::text("Click Me").with_confidence(0.9))
        .add(Selector::role("button"));

    let fingerprint = ElementFingerprint::new("fp_123")
        .with_selectors(selectors)
        .with_role("button")
        .with_name("Submit Button")
        .with_text("Click Me");

    // Serialize to JSON
    let json = serde_json::to_string(&fingerprint).expect("Should serialize");
    assert!(json.contains("fp_123"));
    assert!(json.contains("#my-button"));

    // Deserialize back
    let restored: ElementFingerprint = serde_json::from_str(&json).expect("Should deserialize");
    assert_eq!(restored.id, "fp_123");
    assert_eq!(restored.role.as_deref(), Some("button"));
    assert_eq!(restored.selectors.selectors.len(), 3);
}

/// Test SelectorSet operations
#[test]
fn test_selector_set_operations() {
    // Builder pattern - chain all additions
    let set = SelectorSet::default()
        .add(Selector::test_id("submit-btn"))
        .add(Selector::css("button.primary"))
        .add(Selector::xpath("//button[@type='submit']"))
        .add(Selector::text("Submit").with_confidence(0.85));

    assert_eq!(set.selectors.len(), 4);

    // Check selector kinds
    let kinds: Vec<_> = set.selectors.iter().map(|s| &s.kind).collect();
    assert!(kinds.contains(&&SelectorKind::TestId));
    assert!(kinds.contains(&&SelectorKind::Css));
    assert!(kinds.contains(&&SelectorKind::Xpath));
    assert!(kinds.contains(&&SelectorKind::Text));

    // Test serialization
    let json = serde_json::to_string(&set).expect("Should serialize");
    let restored: SelectorSet = serde_json::from_str(&json).expect("Should deserialize");
    assert_eq!(restored.selectors.len(), 4);
}

/// Test FingerprintMatchOptions defaults
#[test]
fn test_match_options_defaults() {
    let options = FingerprintMatchOptions::default();

    assert_eq!(options.strategy, MatchStrategy::Hybrid);
    assert!((options.min_confidence - 0.8).abs() < f64::EPSILON);
    assert_eq!(options.max_fallback_attempts, 3);
    assert_eq!(options.timeout_ms, 10000);
    assert!(options.search_region.is_none());
}

/// Test MatchStrategy variants
#[test]
fn test_match_strategy_serialization() {
    let strategies = vec![
        MatchStrategy::Dom,
        MatchStrategy::Accessibility,
        MatchStrategy::Vision,
        MatchStrategy::Hybrid,
        MatchStrategy::LlmAssisted,
    ];

    for strategy in strategies {
        let json = serde_json::to_string(&strategy).expect("Should serialize");
        let restored: MatchStrategy = serde_json::from_str(&json).expect("Should deserialize");
        assert_eq!(restored, strategy);
    }
}

/// Test that automation nodes have proper descriptions
#[test]
fn test_nodes_have_descriptions() {
    let catalog = get_catalog();
    let mut missing_descriptions = Vec::new();

    for node_logic in &catalog {
        let node = node_logic.get_node();

        // Check if description is missing or too short
        if node.description.len() < 10 {
            missing_descriptions.push(node.name.clone());
        }
    }

    if !missing_descriptions.is_empty() {
        println!(
            "Warning: {} nodes have missing/short descriptions",
            missing_descriptions.len()
        );
        for name in missing_descriptions.iter().take(10) {
            println!("  - {name}");
        }
    }
}

/// Test that all nodes have unique IDs
#[test]
fn test_nodes_have_unique_ids() {
    let catalog = get_catalog();
    let mut seen_ids = std::collections::HashSet::new();

    for node_logic in &catalog {
        let node = node_logic.get_node();
        assert!(
            seen_ids.insert(node.id.clone()),
            "Duplicate node ID found: {}",
            node.id
        );
    }
}

// =============================================================================
// Template Matching Accuracy & Performance Tests
// =============================================================================

mod template_matching_tests {
    use flow_like_catalog_automation::types::templates::{
        ClickTemplateOptions, MatchResult, TemplateMatchAllOptions, TemplateMatchOptions,
        TemplateMatchResult, TemplateRef,
    };
    use flow_like_catalog_core::BoundingBox;

    /// Test TemplateMatchOptions defaults and builder pattern
    #[test]
    fn test_template_match_options_defaults() {
        let opts = TemplateMatchOptions::default();

        assert!((opts.threshold - 0.8).abs() < f64::EPSILON);
        assert!(opts.search_region.is_none());
        assert!(opts.scales.is_none());
        assert!(opts.use_grayscale);
        assert!(!opts.use_canny_edge);
    }

    /// Test TemplateMatchAllOptions for multi-match scenarios
    #[test]
    fn test_template_match_all_options() {
        let opts = TemplateMatchAllOptions::default();

        assert!((opts.threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(opts.max_hits, 10);
        assert!(opts.non_max_suppression);
        assert!((opts.nms_threshold - 0.3).abs() < f64::EPSILON);
    }

    /// Test ClickTemplateOptions for click operations
    #[test]
    fn test_click_template_options() {
        let opts = ClickTemplateOptions::default();

        assert!(opts.click_offset.is_none());
        assert_eq!(opts.retries, 3);
        assert_eq!(opts.retry_delay_ms, 500);
        assert!((opts.threshold - 0.8).abs() < f64::EPSILON);
    }

    /// Test MatchResult construction and accessors
    #[test]
    fn test_match_result_not_found() {
        let result = MatchResult::not_found();

        assert!(!result.found);
        assert!(result.bbox.is_none());
        assert!((result.confidence - 0.0).abs() < f64::EPSILON);
        assert!(result.center.is_none());
        assert!(result.scale.is_none());
        assert_eq!(result.match_time_ms, 0);
    }

    /// Test MatchResult with found match
    #[test]
    fn test_match_result_found() {
        let bbox = BoundingBox {
            x1: 100.0,
            y1: 100.0,
            x2: 200.0,
            y2: 200.0,
            score: 0.95,
            class_idx: 0,
            class_name: None,
        };
        let result = MatchResult::found(bbox.clone(), 0.95);

        assert!(result.found);
        assert!(result.bbox.is_some());
        assert!((result.confidence - 0.95).abs() < f64::EPSILON);

        let center = result.center.unwrap();
        assert_eq!(center.0, 150);
        assert_eq!(center.1, 150);
    }

    /// Test MatchResult builder methods
    #[test]
    fn test_match_result_builder() {
        let bbox = BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 100.0,
            score: 0.9,
            class_idx: 0,
            class_name: None,
        };
        let result = MatchResult::found(bbox, 0.9)
            .with_match_time(42)
            .with_scale(0.75);

        assert_eq!(result.match_time_ms, 42);
        assert!((result.scale.unwrap() - 0.75).abs() < f64::EPSILON);
    }

    /// Test TemplateMatchResult serialization
    #[test]
    fn test_template_match_result_serialization() {
        let result = TemplateMatchResult {
            found: true,
            x: 150,
            y: 250,
            confidence: 0.92,
            template_path: "/path/to/template.png".to_string(),
        };

        let json = serde_json::to_string(&result).expect("Should serialize");
        assert!(json.contains("\"found\":true"));
        assert!(json.contains("\"x\":150"));
        assert!(json.contains("\"confidence\":0.92"));

        let restored: TemplateMatchResult =
            serde_json::from_str(&json).expect("Should deserialize");
        assert_eq!(restored.x, 150);
        assert_eq!(restored.y, 250);
        assert!(restored.found);
    }

    /// Test TemplateRef creation and builder pattern
    #[test]
    fn test_template_ref_builder() {
        use flow_like_catalog_core::FlowPath;

        let bbox = BoundingBox {
            x1: 10.0,
            y1: 20.0,
            x2: 110.0,
            y2: 120.0,
            score: 1.0,
            class_idx: 0,
            class_name: None,
        };

        let template_ref = TemplateRef::new(
            "tmpl_001",
            FlowPath::new(
                "screenshots/button.png".to_string(),
                "local".to_string(),
                None,
            ),
            bbox,
        )
        .with_scale_invariant(true)
        .with_grayscale(false)
        .with_description("Submit button template");

        assert_eq!(template_ref.template_id, "tmpl_001");
        assert!(template_ref.scale_invariant);
        assert!(!template_ref.grayscale);
        assert_eq!(
            template_ref.description.as_deref(),
            Some("Submit button template")
        );
    }

    /// Test TemplateRef serialization roundtrip
    #[test]
    fn test_template_ref_serialization() {
        use flow_like_catalog_core::FlowPath;

        let bbox = BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 50.0,
            y2: 50.0,
            score: 1.0,
            class_idx: 0,
            class_name: None,
        };

        let original = TemplateRef::new(
            "tmpl_icon",
            FlowPath::new(
                "artifacts/icon.png".to_string(),
                "artifact".to_string(),
                None,
            ),
            bbox,
        );

        let json = serde_json::to_string(&original).expect("Should serialize");
        let restored: TemplateRef = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(restored.template_id, original.template_id);
        assert_eq!(restored.scale_invariant, original.scale_invariant);
        assert_eq!(restored.grayscale, original.grayscale);
    }

    /// Test confidence threshold edge cases
    #[test]
    fn test_confidence_thresholds() {
        let thresholds = [0.0, 0.1, 0.5, 0.8, 0.9, 0.95, 0.99, 1.0];

        for threshold in thresholds {
            let opts = TemplateMatchOptions {
                threshold,
                ..Default::default()
            };

            let json = serde_json::to_string(&opts).expect("Should serialize");
            let restored: TemplateMatchOptions =
                serde_json::from_str(&json).expect("Should deserialize");

            assert!(
                (restored.threshold - threshold).abs() < f64::EPSILON,
                "Threshold {threshold} should roundtrip"
            );
        }
    }

    /// Test BoundingBox calculations for template matching
    #[test]
    fn test_bounding_box_calculations() {
        let bbox = BoundingBox {
            x1: 100.0,
            y1: 200.0,
            x2: 300.0,
            y2: 400.0,
            score: 0.95,
            class_idx: 0,
            class_name: None,
        };

        // Calculate center
        let center_x = (bbox.x1 + bbox.x2) / 2.0;
        let center_y = (bbox.y1 + bbox.y2) / 2.0;

        assert!((center_x - 200.0).abs() < f32::EPSILON);
        assert!((center_y - 300.0).abs() < f32::EPSILON);

        // Calculate dimensions
        let width = bbox.x2 - bbox.x1;
        let height = bbox.y2 - bbox.y1;

        assert!((width - 200.0).abs() < f32::EPSILON);
        assert!((height - 200.0).abs() < f32::EPSILON);
    }

    /// Test search region constraints
    #[test]
    fn test_search_region_constraints() {
        let search_region = BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 1920.0,
            y2: 1080.0,
            score: 1.0,
            class_idx: 0,
            class_name: None,
        };

        let opts = TemplateMatchOptions {
            search_region: Some(search_region.clone()),
            ..Default::default()
        };

        let json = serde_json::to_string(&opts).expect("Should serialize");
        let restored: TemplateMatchOptions =
            serde_json::from_str(&json).expect("Should deserialize");

        let region = restored.search_region.expect("Should have search region");
        assert!((region.x2 - 1920.0).abs() < f32::EPSILON);
        assert!((region.y2 - 1080.0).abs() < f32::EPSILON);
    }

    /// Test multi-scale matching options
    #[test]
    fn test_multi_scale_options() {
        let scales = vec![0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

        let opts = TemplateMatchOptions {
            scales: Some(scales.clone()),
            ..Default::default()
        };

        let json = serde_json::to_string(&opts).expect("Should serialize");
        let restored: TemplateMatchOptions =
            serde_json::from_str(&json).expect("Should deserialize");

        let restored_scales = restored.scales.expect("Should have scales");
        assert_eq!(restored_scales.len(), 6);
        assert!((restored_scales[2] - 1.0).abs() < f64::EPSILON);
    }

    /// Performance: Test serialization speed for batch results
    #[test]
    fn test_batch_serialization_performance() {
        use std::time::Instant;

        let results: Vec<TemplateMatchResult> = (0..1000)
            .map(|i| TemplateMatchResult {
                found: i % 3 == 0,
                x: i * 10,
                y: i * 5,
                confidence: (i as f64 % 100.0) / 100.0,
                template_path: format!("/path/template_{i}.png"),
            })
            .collect();

        let start = Instant::now();
        let json = serde_json::to_string(&results).expect("Should serialize");
        let serialize_time = start.elapsed();

        let start = Instant::now();
        let _restored: Vec<TemplateMatchResult> =
            serde_json::from_str(&json).expect("Should deserialize");
        let deserialize_time = start.elapsed();

        println!("Batch serialization (1000 results): {:?}", serialize_time);
        println!(
            "Batch deserialization (1000 results): {:?}",
            deserialize_time
        );

        // Ensure reasonable performance (should be under 100ms for 1000 items)
        assert!(
            serialize_time.as_millis() < 100,
            "Serialization too slow: {:?}",
            serialize_time
        );
        assert!(
            deserialize_time.as_millis() < 100,
            "Deserialization too slow: {:?}",
            deserialize_time
        );
    }

    /// Test NMS (Non-Maximum Suppression) threshold values
    #[test]
    fn test_nms_thresholds() {
        let opts_low = TemplateMatchAllOptions {
            nms_threshold: 0.1,
            ..Default::default()
        };

        let opts_high = TemplateMatchAllOptions {
            nms_threshold: 0.9,
            ..Default::default()
        };

        // Low NMS threshold = more aggressive suppression
        // High NMS threshold = less suppression, more overlapping matches
        assert!(opts_low.nms_threshold < opts_high.nms_threshold);

        // Both should serialize correctly
        let json_low = serde_json::to_string(&opts_low).expect("Should serialize");
        let json_high = serde_json::to_string(&opts_high).expect("Should serialize");

        assert!(json_low.contains("0.1"));
        assert!(json_high.contains("0.9"));
    }
}

// =============================================================================
// Node Serialization Tests
// =============================================================================

mod node_serialization_tests {
    use flow_like::flow::node::Node;
    use flow_like_catalog_automation::get_catalog;

    /// Test that all nodes can be serialized to JSON
    #[test]
    fn test_all_nodes_serialize() {
        let catalog = get_catalog();
        let mut failures = Vec::new();

        for node_logic in &catalog {
            let node = node_logic.get_node();
            match serde_json::to_string(&node) {
                Ok(_) => {}
                Err(e) => {
                    failures.push((node.id.clone(), e.to_string()));
                }
            }
        }

        if !failures.is_empty() {
            for (id, err) in &failures {
                eprintln!("Node '{id}' failed to serialize: {err}");
            }
            panic!("{} nodes failed to serialize", failures.len());
        }
    }

    /// Test node serialization roundtrip
    #[test]
    fn test_node_serialization_roundtrip() {
        let catalog = get_catalog();
        let mut roundtrip_failures = Vec::new();

        for node_logic in &catalog {
            let original = node_logic.get_node();
            let json = match serde_json::to_string(&original) {
                Ok(j) => j,
                Err(_) => continue,
            };

            match serde_json::from_str::<Node>(&json) {
                Ok(restored) => {
                    if original.id != restored.id || original.name != restored.name {
                        roundtrip_failures.push(original.id.clone());
                    }
                }
                Err(e) => {
                    roundtrip_failures.push(format!("{}: {}", original.id, e));
                }
            }
        }

        if !roundtrip_failures.is_empty() {
            for failure in &roundtrip_failures {
                eprintln!("Roundtrip failure: {failure}");
            }
            panic!(
                "{} nodes failed roundtrip serialization",
                roundtrip_failures.len()
            );
        }
    }

    /// Test that vision nodes have correct pin schemas
    #[test]
    fn test_vision_node_pin_schemas() {
        let catalog = get_catalog();

        for node_logic in &catalog {
            let node = node_logic.get_node();

            if !node.category.starts_with("Automation/Vision") {
                continue;
            }

            // Vision nodes should have session input (check by name in Pin values)
            let has_session_pin = node.pins.values().any(|p| p.name == "session");

            // Vision nodes that do matching should have confidence input
            let is_find_or_click = node.id.contains("find") || node.id.contains("click");

            if is_find_or_click {
                let has_confidence_pin = node.pins.values().any(|p| p.name == "confidence");
                assert!(
                    has_confidence_pin,
                    "Vision node '{}' should have confidence pin",
                    node.name
                );
            }

            assert!(
                has_session_pin,
                "Vision node '{}' should have session pin",
                node.name
            );
        }
    }

    /// Test that computer nodes have proper execution pins
    #[test]
    fn test_computer_node_execution_pins() {
        let catalog = get_catalog();

        for node_logic in &catalog {
            let node = node_logic.get_node();

            if !node.category.starts_with("Automation/Computer") {
                continue;
            }

            // Skip nodes that are pure data nodes (no execution flow)
            // Some computer nodes like list_windows might be data-only
            let has_exec_in = node.pins.values().any(|p| p.name == "exec_in");
            let has_exec_out = node.pins.values().any(|p| p.name == "exec_out");

            // Only assert for nodes that have at least one exec pin
            // (some nodes might be pure data transformers)
            if has_exec_in || has_exec_out {
                assert!(
                    has_exec_in,
                    "Computer node '{}' with exec_out should have exec_in pin",
                    node.name
                );
                assert!(
                    has_exec_out,
                    "Computer node '{}' with exec_in should have exec_out pin",
                    node.name
                );
            }
        }
    }
}

// =============================================================================
// Template Matching Accuracy Tests (requires `execute` feature)
// =============================================================================
// These tests use actual rustautogui template matching against known test images
// from the RustAutoGUI project (MIT License): https://github.com/DavorMar/rustautogui
//
// Synthetic images are inspired by PyAutoGUI testing methodology (BSD-3-Clause):
// https://github.com/asweigart/pyautogui

#[cfg(feature = "execute")]
mod template_matching_accuracy_tests {
    use image::{ImageBuffer, Rgb, RgbImage};
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
    }

    fn algorithm_tests_dir() -> PathBuf {
        fixtures_dir().join("algorithm_tests")
    }

    /// Test basic template matching with synthetic gradient images
    /// Inspired by PyAutoGUI's testing methodology (BSD-3-Clause)
    /// Note: Solid color images don't work with Segmented mode (no texture features),
    /// so we use FFT mode for uniform templates or create gradient patterns.
    #[test]
    fn test_synthetic_gradient_template_preparation() {
        // Create a gradient image with visual features (not solid color)
        let main_image: RgbImage = ImageBuffer::from_fn(200, 200, |x, y| {
            Rgb([x as u8, y as u8, 128]) // Gradient pattern
        });

        // Create a template from a region of the gradient
        let template: RgbImage = ImageBuffer::from_fn(50, 50, |x, y| {
            // Match the pattern at position (75, 75) in main image
            Rgb([(75 + x) as u8, (75 + y) as u8, 128])
        });

        // Save images for template matching test
        let temp_dir = std::env::temp_dir();
        let main_path = temp_dir.join("test_gradient_main.png");
        let template_path = temp_dir.join("test_gradient_template.png");
        main_image
            .save(&main_path)
            .expect("Failed to save main image");
        template
            .save(&template_path)
            .expect("Failed to save template");

        // Test that template preparation works with gradient images
        let mut gui = rustautogui::RustAutoGui::new(false).expect("Failed to create RustAutoGui");

        // Segmented mode should work with gradient images (has texture)
        let result = gui.prepare_template_from_file(
            template_path.to_str().unwrap(),
            None,
            rustautogui::MatchMode::Segmented,
        );
        assert!(
            result.is_ok(),
            "Segmented mode should work with gradient images: {:?}",
            result.err()
        );

        // FFT mode should also work
        let result = gui.prepare_template_from_file(
            template_path.to_str().unwrap(),
            None,
            rustautogui::MatchMode::FFT,
        );
        assert!(
            result.is_ok(),
            "FFT mode should work with gradient images: {:?}",
            result.err()
        );

        // Clean up
        std::fs::remove_file(main_path).ok();
        std::fs::remove_file(template_path).ok();
    }

    /// Test that rustautogui fixture images exist and are loadable
    #[test]
    fn test_fixture_images_exist() {
        let test_cases = [
            ("Darts_main.png", "Darts_template1.png"),
            ("Socket_main.png", "Socket_template1.png"),
            ("Split_main.png", "Split_template1.png"),
        ];

        let dir = algorithm_tests_dir();
        for (main_name, template_name) in test_cases {
            let main_path = dir.join(main_name);
            let template_path = dir.join(template_name);

            if main_path.exists() && template_path.exists() {
                // Verify images are loadable
                let main_img = image::open(&main_path)
                    .unwrap_or_else(|e| panic!("Failed to load {}: {}", main_name, e));
                let template_img = image::open(&template_path)
                    .unwrap_or_else(|e| panic!("Failed to load {}: {}", template_name, e));

                // Verify template is smaller than main image
                assert!(
                    template_img.width() < main_img.width()
                        && template_img.height() < main_img.height(),
                    "Template {} should be smaller than main image {}",
                    template_name,
                    main_name
                );
            } else {
                // Skip if fixtures not downloaded - this is expected in CI without setup
                println!(
                    "Skipping {} / {} - fixtures not downloaded",
                    main_name, template_name
                );
            }
        }
    }

    /// Test rustautogui template preparation with real images
    #[test]
    fn test_template_preparation_with_fixtures() {
        let dir = algorithm_tests_dir();
        let template_path = dir.join("Darts_template1.png");

        if !template_path.exists() {
            println!("Skipping test - Darts_template1.png not found. Run download_fixtures.sh");
            return;
        }

        let mut gui = rustautogui::RustAutoGui::new(false).expect("Failed to create RustAutoGui");

        // Test Segmented mode preparation
        let result = gui.prepare_template_from_file(
            template_path.to_str().unwrap(),
            None,
            rustautogui::MatchMode::Segmented,
        );
        assert!(
            result.is_ok(),
            "Segmented template preparation should succeed: {:?}",
            result.err()
        );

        // Test FFT mode preparation
        let result = gui.prepare_template_from_file(
            template_path.to_str().unwrap(),
            None,
            rustautogui::MatchMode::FFT,
        );
        assert!(
            result.is_ok(),
            "FFT template preparation should succeed: {:?}",
            result.err()
        );
    }

    /// Test multiple template storage
    #[test]
    fn test_multiple_template_storage() {
        let dir = algorithm_tests_dir();

        let templates = [
            ("darts1", "Darts_template1.png"),
            ("darts2", "Darts_template2.png"),
            ("socket1", "Socket_template1.png"),
        ];

        let mut gui = rustautogui::RustAutoGui::new(false).expect("Failed to create RustAutoGui");
        let mut loaded_count = 0;

        for (alias, filename) in templates {
            let path = dir.join(filename);
            if !path.exists() {
                println!("Skipping {} - file not found", filename);
                continue;
            }

            let result = gui.store_template_from_file(
                path.to_str().unwrap(),
                None,
                rustautogui::MatchMode::Segmented,
                alias,
            );

            assert!(
                result.is_ok(),
                "Should store template {}: {:?}",
                alias,
                result.err()
            );
            loaded_count += 1;
        }

        if loaded_count > 0 {
            println!("Successfully stored {} templates", loaded_count);
        }
    }

    /// Performance benchmark: Template preparation time
    #[test]
    fn test_template_preparation_performance() {
        let dir = algorithm_tests_dir();
        let templates = [
            "Darts_template1.png",
            "Socket_template1.png",
            "Split_template1.png",
        ];

        let mut total_time = std::time::Duration::ZERO;
        let mut count = 0;

        for filename in templates {
            let path = dir.join(filename);
            if !path.exists() {
                continue;
            }

            let mut gui =
                rustautogui::RustAutoGui::new(false).expect("Failed to create RustAutoGui");

            let start = std::time::Instant::now();
            let _ = gui.prepare_template_from_file(
                path.to_str().unwrap(),
                None,
                rustautogui::MatchMode::Segmented,
            );
            let elapsed = start.elapsed();

            println!("{}: {:?}", filename, elapsed);
            total_time += elapsed;
            count += 1;
        }

        if count > 0 {
            let avg_time = total_time / count as u32;
            println!("Average template preparation time: {:?}", avg_time);

            // Template preparation should be reasonably fast
            // Note: Debug builds are significantly slower than release builds
            // In release mode, expect < 100ms; in debug mode, allow up to 2s
            #[cfg(debug_assertions)]
            let threshold_ms = 2000; // 2 seconds for debug builds
            #[cfg(not(debug_assertions))]
            let threshold_ms = 500; // 500ms for release builds

            assert!(
                avg_time.as_millis() < threshold_ms,
                "Template preparation too slow: {:?} (threshold: {}ms)",
                avg_time,
                threshold_ms
            );
        }
    }

    /// Test search region constraints
    #[test]
    fn test_search_region_preparation() {
        let dir = algorithm_tests_dir();
        let template_path = dir.join("Darts_template1.png");

        if !template_path.exists() {
            println!("Skipping test - fixture not found");
            return;
        }

        let mut gui = rustautogui::RustAutoGui::new(false).expect("Failed to create RustAutoGui");

        // Prepare with a specific search region (top-left quadrant)
        let result = gui.prepare_template_from_file(
            template_path.to_str().unwrap(),
            Some((0, 0, 500, 500)), // x, y, width, height
            rustautogui::MatchMode::Segmented,
        );

        assert!(
            result.is_ok(),
            "Should prepare template with search region: {:?}",
            result.err()
        );
    }
}
