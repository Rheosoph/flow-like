use super::*;
use serde_json::json;

const HUB: &str = "models.example";

fn model(id: &str, assets: Value) -> Value {
    json!({
        "id": id,
        "ref": id,
        "type": "Forecasting",
        "file_name": "model.safetensors",
        "parameters": {
            "model_type": "chronos2",
            "provider": {"provider_name": "local:forecast"},
            "assets": assets
        }
    })
}

fn config() -> Value {
    json!({"id": "config-plan", "ref": "config", "type": "Config", "file_name": "config.json"})
}

fn stored(id: &str, hub: &str) -> Bit {
    Bit {
        id: id.into(),
        hub: hub.into(),
        ..Bit::default()
    }
}

#[test]
fn hosted_deduplication_refreshes_parameters_at_the_returned_id() {
    let bundle = Spec {
        bits: vec![json!({
            "id": "openrouter-standard-chat",
            "type": "Vlm",
            "parameters": {
                "provider": {"provider_name": "hosted:openrouter", "model_id": "vendor/new-model"},
                "pricing": {"input_micro_usd_per_million_tokens": 100_000, "output_micro_usd_per_million_tokens": 400_000}
            },
            "meta": {"en": {"name": "Updated hosted chat"}}
        })],
    };
    let planned = plan(&bundle, HUB).unwrap();
    let payload = planned[0].push_payload(&BTreeMap::new(), HUB).unwrap();
    let existing = Bit {
        id: "server-generated-id".into(),
        bit_type: BitTypes::Vlm,
        hub: "canonical.example".into(),
        hash: "openrouter-standard-chat".into(),
        dependency_tree_hash: "server-tree".into(),
        parameters: json!({"provider": {"provider_name": "hosted:openrouter", "model_id": "vendor/old-model"}}),
        ..Bit::default()
    };
    let update = hosted_update_payload(&payload, &existing).unwrap();
    assert_eq!(update["id"], existing.id);
    assert_eq!(update["hub"], existing.hub);
    assert_eq!(update["hash"], existing.hash);
    assert_eq!(
        update["dependency_tree_hash"],
        existing.dependency_tree_hash
    );
    assert_eq!(update["parameters"], payload["parameters"]);
    assert_eq!(planned[0].meta["en"].name, "Updated hosted chat");
    assert!(hosted_update_payload(&update, &existing).is_none());
}

#[test]
fn hosted_refresh_does_not_change_artifact_or_custom_provider_deduplication() {
    let payload = json!({
        "id": "authoring-id", "hash": "", "download_link": null, "file_name": null,
        "dependencies": [],
        "parameters": {"provider": {"provider_name": "hosted:openrouter"}}
    });
    let existing = Bit {
        id: "stored-id".into(),
        hash: "authoring-id".into(),
        ..Bit::default()
    };
    for (key, value) in [
        ("download_link", json!("https://models.example/weights")),
        ("file_name", json!("weights.gguf")),
        ("dependencies", json!(["models.example:tokenizer"])),
        ("hash", json!("explicit-content-hash")),
        (
            "parameters",
            json!({"provider": {"provider_name": "custom:openrouter"}}),
        ),
    ] {
        let mut altered = payload.clone();
        altered[key] = value;
        assert!(
            hosted_update_payload(&altered, &existing).is_none(),
            "{key}"
        );
    }
    for altered in [
        Bit {
            hash: "unrelated-identity".into(),
            ..existing.clone()
        },
        Bit {
            download_link: Some("https://cdn.example/file".into()),
            ..existing.clone()
        },
        Bit {
            file_name: Some("weights.gguf".into()),
            ..existing.clone()
        },
        Bit {
            dependencies: vec!["models.example:tokenizer".into()],
            ..existing.clone()
        },
    ] {
        assert!(hosted_update_payload(&payload, &altered).is_none());
    }
}

#[test]
fn openrouter_catalog_contains_publishable_hosted_media_and_embeddings() {
    let bundle = load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("specs/hosted-openrouter.json"),
    )
    .unwrap();
    let planned = plan(&bundle, HUB).unwrap();
    let repeated = plan(&bundle, HUB).unwrap();
    assert_eq!(planned.len(), 127);
    assert_eq!(
        planned.iter().map(|entry| &entry.id).collect::<Vec<_>>(),
        repeated.iter().map(|entry| &entry.id).collect::<Vec<_>>(),
        "catalog IDs must survive repeated publication plans"
    );
    let mut kinds = BTreeMap::<String, usize>::new();
    let mut ids = HashSet::new();
    let mut models = HashSet::new();
    for entry in &planned {
        assert!(ids.insert(&entry.id), "duplicate catalog ID");
        let payload = entry.push_payload(&BTreeMap::new(), HUB).unwrap();
        let bit: Bit = serde_json::from_value(payload.clone()).unwrap();
        assert!(bit.download_link.is_none() && bit.file_name.is_none());
        assert!(bit.dependencies.is_empty());
        assert!(bit.size.is_none_or(|size| size == 0));
        assert!(
            bit.model_slug.is_none(),
            "evaluation IDs need existing registry rows"
        );
        let provider = bit.parameters.get("provider").unwrap();
        assert!(is_hosted_provider_name(
            provider["provider_name"].as_str().unwrap()
        ));
        assert!(provider.get("api_key").is_none());
        assert!(provider.get("endpoint").is_none());
        *kinds.entry(entry.bit_type.clone()).or_default() += 1;

        let model_id = match bit.bit_type {
            BitTypes::Embedding => {
                assert!(bit.try_to_embedding().is_some());
                let remote = &bit.parameters["remote"];
                assert_eq!(remote["implementation"], "OpenRouter");
                assert!(remote.get("secret_name").is_none_or(Value::is_null));
                assert!(remote.get("endpoint").is_none_or(Value::is_null));
                assert!(
                    bit.parameters["pricing"]["input_micro_usd_per_million_tokens"]
                        .as_u64()
                        .is_some()
                );
                remote["model_id"].as_str().unwrap()
            }
            BitTypes::ImageGeneration => {
                assert_eq!(provider["provider_name"], "hosted:openrouter");
                let pricing = bit.parameters["pricing"].as_object().unwrap();
                assert_eq!(pricing.len(), 1);
                assert!(
                    pricing["request_micro_usd"]
                        .as_u64()
                        .is_some_and(|price| price > 0)
                );
                provider["model_id"].as_str().unwrap()
            }
            BitTypes::Stt | BitTypes::Tts => {
                assert_eq!(provider["provider_name"], "hosted:openrouter");
                assert!(provider.get("api_surface").is_none_or(Value::is_null));
                let pricing = bit.parameters["pricing"].as_object().unwrap();
                assert_eq!(pricing.len(), 1);
                let (unit, amount) = pricing.iter().next().unwrap();
                assert!(amount.as_u64().is_some());
                assert!(match bit.bit_type {
                    BitTypes::Stt => unit == "input_micro_usd_per_million_seconds",
                    BitTypes::Tts => matches!(
                        unit.as_str(),
                        "input_micro_usd_per_million_characters"
                            | "input_micro_usd_per_million_bytes"
                    ),
                    _ => false,
                });
                provider["model_id"].as_str().unwrap()
            }
            _ => panic!("Unexpected hosted catalog type {}", entry.bit_type),
        };
        assert!(model_id.contains('/') && !model_id.starts_with("@preset/"));
        assert!(models.insert((entry.bit_type.clone(), model_id.to_string())));
        let meta = entry.meta.get("en").unwrap();
        assert!(!meta.name.trim().is_empty() && !meta.description.trim().is_empty());
        assert!(meta.website.is_some() || meta.docs_url.is_some());
    }
    assert_eq!(
        kinds,
        BTreeMap::from([
            ("Embedding".into(), 38),
            ("ImageGeneration".into(), 31),
            ("Stt".into(), 27),
            ("Tts".into(), 31),
        ])
    );
}

#[test]
fn asset_aliases_order_publication_and_use_returned_ids_in_both_locations() {
    let mut forecast = model(
        "model-plan",
        json!([
            {"bit": "@config", "relative_path": "config.json"},
            {"bit": "@weights", "relative_path": "model-00001-of-00002.safetensors"},
            {"bit": "external.example:remote-shard", "relative_path": "model-00002-of-00002.safetensors"}
        ]),
    );
    forecast["dependencies"] = json!(["@config"]);
    let planned = plan(
        &Spec {
            bits: vec![
                forecast,
                config(),
                json!({"id": "weights-plan", "ref": "weights", "type": "File"}),
            ],
        },
        HUB,
    )
    .unwrap();
    assert_eq!(
        planned
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["config-plan", "weights-plan", "model-plan"]
    );
    let root = planned.last().unwrap();
    assert_eq!(
        root.dependencies.len(),
        3,
        "explicit and asset dependencies deduplicate"
    );
    let returned = BTreeMap::from([
        (
            "config-plan".into(),
            stored("existing-config", "canonical.example"),
        ),
        ("weights-plan".into(), stored("new-weights", "")),
    ]);
    let payload = root.push_payload(&returned, HUB).unwrap();
    assert_eq!(
        payload["dependencies"],
        json!([
            "canonical.example:existing-config",
            "models.example:new-weights",
            "external.example:remote-shard"
        ])
    );
    let assets = payload["parameters"]["assets"].as_array().unwrap();
    assert_eq!(assets[0]["bit"], "canonical.example:existing-config");
    assert_eq!(assets[1]["bit"], "models.example:new-weights");
    assert_eq!(assets[2]["bit"], "external.example:remote-shard");
    assert_eq!(assets[0]["relative_path"], "config.json");
    assert_eq!(payload["parameters"]["model_type"], "chronos2");
    assert_eq!(root.payload["parameters"]["assets"][0]["bit"], "@config");
    assert!(root.push_payload(&BTreeMap::new(), HUB).is_err());
}

#[test]
fn remote_asset_references_preserve_their_hub_and_qualify_bare_ids() {
    let planned = plan(
        &Spec {
            bits: vec![model(
                "model",
                json!([
                    {"bit": "existing-config", "relative_path": "config.json"},
                    {"bit": "other.example:weights", "relative_path": "weights.safetensors"}
                ]),
            )],
        },
        HUB,
    )
    .unwrap();
    let payload = planned[0].push_payload(&BTreeMap::new(), HUB).unwrap();
    assert_eq!(
        payload["parameters"]["assets"][0]["bit"],
        "models.example:existing-config"
    );
    assert_eq!(
        payload["parameters"]["assets"][1]["bit"],
        "other.example:weights"
    );
    assert_eq!(
        payload["dependencies"],
        json!(["models.example:existing-config", "other.example:weights"])
    );
}

#[test]
fn unknown_asset_aliases_and_asset_dependency_cycles_fail_before_publication() {
    let undefined = plan(
        &Spec {
            bits: vec![model(
                "model",
                json!([{"bit": "@missing", "relative_path": "config.json"}]),
            )],
        },
        HUB,
    );
    assert!(undefined.err().unwrap().to_string().contains("@missing"));
    let cyclic = plan(
        &Spec {
            bits: vec![
                model(
                    "first",
                    json!([{"bit": "@second", "relative_path": "config.json"}]),
                ),
                model(
                    "second",
                    json!([{"bit": "@first", "relative_path": "config.json"}]),
                ),
            ],
        },
        HUB,
    );
    assert!(
        cyclic
            .err()
            .unwrap()
            .to_string()
            .contains("dependency cycle")
    );
}

#[test]
fn root_file_mappings_do_not_create_self_dependencies_or_predict_new_root_ids() {
    for root_reference in [
        "@root",
        "root",
        "models.example:root",
        "https://models.example:root",
    ] {
        let planned = plan(
            &Spec {
                bits: vec![
                    model(
                        "root",
                        json!([
                            {"bit": "@config", "relative_path": "config.json"},
                            {"bit": root_reference, "relative_path": "model.safetensors"}
                        ]),
                    ),
                    config(),
                ],
            },
            HUB,
        )
        .unwrap();
        let root = planned.last().unwrap();
        assert_eq!(
            root.dependencies,
            vec![Dependency::Local("config-plan".into())]
        );
        assert_eq!(
            root.payload["parameters"]["assets"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(root.payload["file_name"], "model.safetensors");
    }
    let renamed = plan(
        &Spec {
            bits: vec![
                model(
                    "root",
                    json!([
                        {"bit": "@config", "relative_path": "config.json"},
                        {"bit": "@root", "relative_path": "renamed.safetensors"}
                    ]),
                ),
                config(),
            ],
        },
        HUB,
    );
    assert!(
        renamed
            .err()
            .unwrap()
            .to_string()
            .contains("root file_name")
    );
}

#[test]
fn pulled_asset_aliases_republish_without_reusing_old_dependency_identities() {
    let config = Bit {
        bit_type: BitTypes::Config,
        ..stored("old-config", HUB)
    };
    let root = Bit {
        bit_type: BitTypes::Forecasting,
        file_name: Some("model.safetensors".into()),
        parameters: json!({
            "model_type": "timestral",
            "assets": [
                {"bit": "models.example:old-config", "relative_path": "config.json"},
                {"bit": "foreign.example:old-config", "relative_path": "foreign.safetensors"},
                {"bit": "old-model", "relative_path": "model.safetensors"}
            ]
        }),
        ..stored("old-model", HUB)
    };
    let exported = from_bits(&[config, root]);
    assert_eq!(
        exported.bits[1]["parameters"]["assets"][0]["bit"],
        "@config-0"
    );
    assert_eq!(
        exported.bits[1]["parameters"]["assets"][1]["bit"],
        "foreign.example:old-config"
    );
    assert_eq!(
        exported.bits[1]["parameters"]["assets"][2]["bit"],
        "@forecasting-1"
    );
    let planned = plan(&exported, HUB).unwrap();
    let payload = planned[1]
        .push_payload(
            &BTreeMap::from([("old-config".into(), stored("returned-config", HUB))]),
            HUB,
        )
        .unwrap();
    assert_eq!(
        payload["parameters"]["assets"][0]["bit"],
        "models.example:returned-config"
    );
    assert_eq!(payload["parameters"]["assets"].as_array().unwrap().len(), 2);
}

#[test]
fn forecasting_validation_runs_before_any_hub_write() {
    for assets in [
        json!([{"bit": "config", "relative_path": "../config.json"}]),
        json!([{"bit": "config", "relative_path": "weights.safetensors"}]),
        json!([{"bit": "config", "relative_path": "config.json"}, {"bit": "another", "relative_path": "config.json"}]),
    ] {
        assert!(
            plan(
                &Spec {
                    bits: vec![model("model", assets)]
                },
                HUB
            )
            .is_err()
        );
    }
}

#[test]
fn release_catalog_pins_sources_and_resolves_every_checkpoint_and_license_asset() {
    let bundle =
        load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("specs/forecasting.json"))
            .unwrap();
    let planned = plan(&bundle, HUB).unwrap();
    assert_eq!(planned.len(), 15);
    let mut returned = BTreeMap::new();
    let mut families = BTreeMap::<String, usize>::new();
    for (index, entry) in planned.iter().enumerate() {
        let payload = entry.push_payload(&returned, HUB).unwrap();
        let source = payload["download_link"].as_str().unwrap();
        let pieces = source.split('/').collect::<Vec<_>>();
        let revision = match pieces[2] {
            "huggingface.co" => {
                let resolve = pieces.iter().position(|part| *part == "resolve").unwrap();
                let revision = pieces[resolve + 1];
                assert_eq!(payload["version"], revision);
                revision
            }
            "raw.githubusercontent.com" => pieces[5],
            other => panic!("Unexpected release download host: {other}"),
        };
        assert_eq!(revision.len(), 40, "release URLs must pin complete commits");
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
        if entry.bit_type == "Forecasting" {
            let parameters: ForecastingModelParameters =
                serde_json::from_value(payload["parameters"].clone()).unwrap();
            parameters.validate().unwrap();
            let family = payload["parameters"]["model_type"].as_str().unwrap();
            *families.entry(family.into()).or_default() += 1;
            assert_eq!(payload["parameters"]["provider"]["version"], revision);
            for name in ["config.json", "LICENSE"] {
                let asset = parameters
                    .assets
                    .iter()
                    .find(|asset| asset.relative_path == name)
                    .unwrap();
                assert!(returned.values().any(|bit: &Bit| {
                    asset.bit == format!("{}:{}", bit.hub, bit.id)
                        && bit.file_name.as_deref() == Some(name)
                }));
                assert!(
                    payload["dependencies"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(asset.bit))
                );
            }
            if family == "timestral" {
                assert_eq!(payload["license"], "Modified MIT (revenue restriction)");
                let metadata = serde_json::to_value(entry.meta.get("en").unwrap()).unwrap();
                let terms = metadata["long_description"].as_str().unwrap();
                assert!(terms.contains("US$20 million") && terms.contains("preceding month"));
                assert!(
                    metadata["tags"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("restricted-license"))
                );
                let license = parameters
                    .assets
                    .iter()
                    .find(|asset| asset.relative_path == "LICENSE")
                    .unwrap();
                let license_bit = returned
                    .values()
                    .find(|bit| license.bit == format!("{}:{}", bit.hub, bit.id))
                    .unwrap();
                assert_eq!(
                    license_bit.license,
                    Some("Modified MIT (revenue restriction)".into())
                );
            }
        }
        let mut saved: Bit = serde_json::from_value(payload).unwrap();
        saved.id = format!("actual-hub-id-{index}");
        saved.hub = "canonical.example".into();
        returned.insert(entry.id.clone(), saved);
    }
    assert_eq!(
        families,
        BTreeMap::from([
            ("timestral".into(), 2),
            ("chronos2".into(), 1),
            ("toto2".into(), 1),
            ("timesfm2_5".into(), 1),
            ("tiny_time_mixer".into(), 1),
        ])
    );
}
