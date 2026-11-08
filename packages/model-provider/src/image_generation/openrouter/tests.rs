use super::*;

fn capabilities(parameters: Value) -> ImageCapabilities {
    ImageCapabilities::from_endpoints(&json!({
        "id": "vendor/image",
        "endpoints": [{
            "provider_tag": "primary", "provider_slug": "primary",
            "supported_parameters": parameters,
        }],
    }))
    .unwrap()
}

fn request(fields: Value, caps: &ImageCapabilities) -> Result<Value> {
    let mut fields = fields.as_object().unwrap().clone();
    fields.insert("prompt".into(), json!("A lighthouse"));
    prepare_request("vendor/image", Value::Object(fields), caps)
}

fn image_parameters() -> Value {
    json!({
        "resolution": {"type":"enum", "values":["1K","2K"]},
        "aspect_ratio": {"type":"enum", "values":["1:1","3:2","2:3","4:3","3:4"]},
        "n": {"type":"range", "min":1,"max":6},
        "seed": {"type":"boolean"},
    })
}

#[test]
fn pixel_sizes_become_supported_tiers_and_exact_ratios() {
    let caps = capabilities(image_parameters());
    for (size, ratio, resolution) in [
        ("1024x1024", "1:1", "1K"),
        ("1536x1024", "3:2", "1K"),
        ("1024x1536", "2:3", "1K"),
        ("2048x2048", "1:1", "2K"),
    ] {
        let body = request(json!({"size":size,"seed":0}), &caps).unwrap();
        assert_eq!(body["resolution"], resolution);
        assert_eq!(body["aspect_ratio"], ratio);
        assert_eq!(body["seed"], 0);
        assert!(body.get("size").is_none());
        assert!(body.get("output_format").is_none());
        assert_eq!(body["provider"]["only"], json!(["primary"]));
    }
}

#[test]
fn openai_style_capabilities_need_no_resolution_and_keep_native_format() {
    let caps = capabilities(json!({
        "aspect_ratio":{"type":"enum","values":["1:1","3:2","2:3","auto"]},
        "output_format":{"type":"enum","values":["png","jpeg","webp"]},
        "quality":{"type":"enum","values":["auto","low","medium","high"]},
        "background":{"type":"enum","values":["auto","transparent","opaque"]},
        "n":{"type":"range","min":1,"max":10},
    }));
    let body = request(json!({"size":"1536x1024","quality":"high","n":4}), &caps).unwrap();
    assert_eq!(body["aspect_ratio"], "3:2");
    assert_eq!(body["quality"], "high");
    assert_eq!(body["n"], 4);
    assert!(body.get("size").is_none());
    assert!(body.get("resolution").is_none());
    assert!(body.get("output_format").is_none());
}

#[test]
fn google_and_flux_shapes_only_receive_the_options_they_advertise() {
    let google = capabilities(json!({
        "resolution":{"type":"enum","values":["1K","2K","4K"]},
        "aspect_ratio":{"type":"enum","values":["1:1","3:2","2:3"]},
        "n":{"type":"range","min":1,"max":1},
    }));
    let body = request(
        json!({"size":"1024x1536","quality":"auto","background":"auto"}),
        &google,
    )
    .unwrap();
    assert_eq!(body["resolution"], "1K");
    assert_eq!(body["aspect_ratio"], "2:3");
    assert!(body.get("quality").is_none());
    assert!(body.get("background").is_none());
    assert!(request(json!({"size":"1024x1024","seed":42}), &google).is_err());
    let flux = capabilities(json!({
        "resolution":{"type":"enum","values":["768","1K","1.5K","2K","4K"]},
        "aspect_ratio":{"type":"enum","values":["1:1","3:2","2:3"]},
        "output_format":{"type":"enum","values":["jpeg","png"]},
        "n":{"type":"range","min":1,"max":1},
    }));
    let body = request(json!({"size":"1536x1024","output_format":"jpeg"}), &flux).unwrap();
    assert_eq!(body["resolution"], "1.5K");
    assert_eq!(body["output_format"], "jpeg");
    assert!(request(json!({"n":2}), &flux).is_err());
}

#[test]
fn minimal_models_receive_no_invented_knobs() {
    let caps = capabilities(json!({}));
    let body = request(
        json!({"size":"1024x1024","n":1,"quality":"auto","background":"auto"}),
        &caps,
    )
    .unwrap();
    assert_eq!(
        body,
        json!({
            "model":"vendor/image", "prompt":"A lighthouse", "stream":false,
            "provider":{"only":["primary"]},
        })
    );
    for fields in [
        json!({"n":2}),
        json!({"seed":42}),
        json!({"background":"transparent"}),
        json!({"output_format":"png"}),
    ] {
        assert!(request(fields, &caps).is_err());
    }
}

#[test]
fn native_pixel_sizes_are_sent_alone_and_conflicts_fail_before_generation() {
    let mut params = image_parameters();
    params["size"] = json!({"type":"boolean"});
    let caps = capabilities(params);
    let body = request(json!({"size":"1536x1024","aspect_ratio":"3:2"}), &caps).unwrap();
    assert_eq!(body["size"], "1536x1024");
    assert!(body.get("aspect_ratio").is_none());
    assert!(body.get("resolution").is_none());
    assert!(request(json!({"size":"1536x1024","aspect_ratio":"4:3"}), &caps).is_err());
    assert!(request(json!({"size":"1536x1024","resolution":"1K"}), &caps).is_err());
}

#[test]
fn only_inferred_ratios_can_choose_a_nearby_shape() {
    let caps = capabilities(json!({
        "aspect_ratio":{"type":"enum","values":["1:1","4:3","3:4","16:9","9:16"]},
    }));
    assert_eq!(
        request(json!({"size":"1536x1024"}), &caps).unwrap()["aspect_ratio"],
        "4:3"
    );
    assert_eq!(
        request(json!({"size":"1024x1536"}), &caps).unwrap()["aspect_ratio"],
        "3:4"
    );
    assert!(request(json!({"aspect_ratio":"3:2"}), &caps).is_err());
    let square = capabilities(json!({"aspect_ratio":{"type":"enum","values":["1:1"]}}));
    assert!(request(json!({"size":"1536x1024"}), &square).is_err());
    let landscape = capabilities(json!({"aspect_ratio":{"type":"enum","values":["4:3"]}}));
    assert!(request(json!({"size":"1024x1024"}), &landscape).is_err());
}

#[test]
fn resolution_never_exceeds_the_2k_bound() {
    let caps = capabilities(json!({
        "resolution":{"type":"enum","values":["4K","2K"]},
        "aspect_ratio":{"type":"enum","values":["1:1"]},
    }));
    assert_eq!(
        request(json!({"size":"2048x2048"}), &caps).unwrap()["resolution"],
        "2K"
    );
    for fields in [
        json!({"size":"4096x4096"}),
        json!({"size":"4K"}),
        json!({"resolution":"4K"}),
        json!({"size":"-1x1024"}),
    ] {
        assert!(request(fields, &caps).is_err());
    }
    let large = capabilities(json!({"resolution":{"type":"enum","values":["4K"]}}));
    assert!(request(json!({"size":"2048x2048"}), &large).is_err());
    assert!(request(json!({"size":"1024x1024","resolution":"2K"}), &caps).is_err());
    assert!(request(json!({"size":"1K","resolution":"2K"}), &caps).is_err());
}

#[test]
fn endpoint_selection_prefers_honoring_resolution_over_dropping_it() {
    let caps = ImageCapabilities::from_endpoints(&json!({
        "id":"vendor/image", "endpoints":[
            {"provider_tag":"no-resolution","supported_parameters":{
                "aspect_ratio":{"type":"enum","values":["3:2"]},
            }},
            {"provider_tag":"with-resolution","supported_parameters":{
                "aspect_ratio":{"type":"enum","values":["3:2"]},
                "resolution":{"type":"enum","values":["1K","2K"]},
            }},
        ],
    }))
    .unwrap();
    let body = request(json!({"size":"1536x1024"}), &caps).unwrap();
    assert_eq!(body["resolution"], "1K");
    assert_eq!(body["provider"]["only"], json!(["with-resolution"]));
}

#[test]
fn reference_only_and_vector_only_models_are_refused_before_spending() {
    let edit = capabilities(json!({"input_references":{"type":"range","min":1,"max":3}}));
    assert!(request(json!({}), &edit).is_err());
    let vector = capabilities(json!({"output_format":{"type":"enum","values":["svg"]}}));
    assert!(request(json!({}), &vector).is_err());
    let mixed = capabilities(json!({"output_format":{"type":"enum","values":["gif","png"]}}));
    assert_eq!(request(json!({}), &mixed).unwrap()["output_format"], "png");
    assert!(request(json!({"output_format":"gif"}), &mixed).is_err());
}

#[test]
fn explicit_quality_format_and_transparency_are_preserved_or_refused() {
    let caps = capabilities(json!({
        "output_format":{"type":"enum","values":["jpeg","webp"]},
        "background":{"type":"enum","values":["transparent","opaque"]},
        "quality":{"type":"enum","values":["low","medium"]},
    }));
    assert_eq!(
        request(json!({"background":"transparent"}), &caps).unwrap()["output_format"],
        "webp"
    );
    for fields in [
        json!({"output_format":"png"}),
        json!({"quality":"high"}),
        json!({"background":"transparent","output_format":"jpeg"}),
    ] {
        assert!(request(fields, &caps).is_err());
    }
}

#[test]
fn endpoint_options_are_never_combined_across_a_model_union() {
    let caps = ImageCapabilities::from_endpoints(&json!({
        "id":"vendor/image", "endpoints":[
            {"provider_tag":"a","provider_slug":"a","supported_parameters":{
                "aspect_ratio":{"type":"enum","values":["3:2"]},
                "output_format":{"type":"enum","values":["jpeg"]},
            }},
            {"provider_tag":"b","provider_slug":"b","supported_parameters":{
                "aspect_ratio":{"type":"enum","values":["4:3"]},
                "output_format":{"type":"enum","values":["png"]},
            }},
        ],
    }))
    .unwrap();
    let body = request(json!({"size":"1536x1024"}), &caps).unwrap();
    assert_eq!(body["aspect_ratio"], "3:2");
    assert_eq!(body["provider"]["only"], json!(["a"]));
    assert!(request(json!({"aspect_ratio":"3:2","output_format":"png"}), &caps).is_err());
    let body = request(
        json!({"size":"1536x1024","provider":{"only":["b"],"allow_fallbacks":false}}),
        &caps,
    )
    .unwrap();
    assert_eq!(body["aspect_ratio"], "4:3");
    assert_eq!(
        body["provider"],
        json!({"only":["b"],"allow_fallbacks":false})
    );
    let body = request(json!({"provider":{"ignore":["a"]}}), &caps).unwrap();
    assert_eq!(body["provider"]["only"], json!(["b"]));
    assert!(request(json!({"provider":{"only":["missing"]}}), &caps).is_err());
    assert!(request(json!({"provider":{"order":["a"]}}), &caps).is_err());
}

#[test]
fn unselectable_endpoints_require_compatible_bodies() {
    let caps = ImageCapabilities::from_endpoints(&json!({
        "id":"vendor/image", "endpoints":[
            {"provider_tag":null,"supported_parameters":{"aspect_ratio":{"type":"enum","values":["3:2"]}}},
            {"provider_tag":null,"supported_parameters":{"aspect_ratio":{"type":"enum","values":["4:3"]}}},
        ],
    })).unwrap();
    assert!(request(json!({"size":"1536x1024"}), &caps).is_err());
    assert!(request(json!({}), &caps).unwrap().get("provider").is_none());
}

#[test]
fn a_provider_tag_cannot_select_an_incompatible_endpoint_record() {
    let mut records = json!({
        "id":"vendor/image", "endpoints":[
            {"provider_tag":"shared","supported_parameters":{"aspect_ratio":{"type":"enum","values":["3:2"]}}},
            {"provider_tag":"shared","supported_parameters":{"aspect_ratio":{"type":"enum","values":["4:3"]}}},
        ],
    });
    let caps = ImageCapabilities::from_endpoints(&records).unwrap();
    assert!(request(json!({"size":"1536x1024"}), &caps).is_err());
    records["endpoints"].as_array_mut().unwrap().push(json!({
        "provider_tag":"compatible", "supported_parameters":{"aspect_ratio":{"type":"enum","values":["4:3"]}},
    }));
    let caps = ImageCapabilities::from_endpoints(&records).unwrap();
    let body = request(json!({"size":"1536x1024"}), &caps).unwrap();
    assert_eq!(body["provider"]["only"], json!(["compatible"]));
    assert_eq!(body["aspect_ratio"], "4:3");
}

#[test]
fn discovery_identity_descriptors_and_request_fields_are_checked() {
    let caps = capabilities(image_parameters());
    assert!(prepare_request("vendor/other", json!({"prompt":"x"}), &caps).is_err());
    for fields in [
        json!({"model":"other/model"}),
        json!({"stream":true}),
        json!({"endpoint":"https://evil.invalid"}),
        json!({"size":22}),
    ] {
        assert!(request(fields, &caps).is_err());
    }
    for bad in [
        json!({"id":"vendor/image","endpoints":[]}),
        json!({"id":"vendor/image","endpoints":[{"supported_parameters":{"n":{"type":"range","min":3,"max":1}}}]}),
        json!({"id":"vendor/image","endpoints":[{"supported_parameters":{"size":{"type":"unknown"}}}]}),
    ] {
        assert!(ImageCapabilities::from_endpoints(&bad).is_err());
    }
}

#[test]
fn capability_urls_cannot_escape_the_configured_api_base() {
    assert_eq!(
        capabilities_url("https://openrouter.ai", "qwen/qwen-image-3").unwrap(),
        "https://openrouter.ai/api/v1/images/models/qwen/qwen-image-3/endpoints"
    );
    assert_eq!(
        capabilities_url("https://example.test/custom/v1/", "vendor/image:variant").unwrap(),
        "https://example.test/custom/v1/images/models/vendor/image:variant/endpoints"
    );
    for model in [
        "../image",
        "vendor/..",
        "vendor/a?x=secret",
        "vendor/a#fragment",
        "vendor/%2e%2e",
        "vendor/a/b",
        "//evil.invalid",
        "vendor/a\\b",
    ] {
        assert!(capabilities_url("https://openrouter.ai/api/v1", model).is_err());
    }
    for base in [
        "https://key@openrouter.ai",
        "https://openrouter.ai?key=private",
        "file:///tmp",
        "https://openrouter.ai#fragment",
    ] {
        assert!(capabilities_url(base, "vendor/image").is_err());
    }
}
