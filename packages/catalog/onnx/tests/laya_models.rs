//! Reference parity for the published Laya checkpoint. Set `LAYA_MODEL_DIR` to a
//! downloaded `mizchi/laya-multilingual-onnx` snapshot, then run this test with
//! `--features execute --test laya_models -- --ignored`.

use flow_like_catalog_onnx::laya::{
    LayaConfig, LayaOptions, LayaQuestionType, infer_laya, prepare_laya,
};
use flow_like_model_provider::ml::{
    ort::session::builder::GraphOptimizationLevel, ort_runtime::configured_session_builder,
};
use std::{fs, path::PathBuf};
use tokenizers::Tokenizer;

struct Fixture {
    name: &'static str,
    text: &'static str,
    kind: LayaQuestionType,
    instructions: &'static str,
    criteria: &'static [&'static str],
    input_ids: &'static [i64],
    marker_pos: &'static [i64],
    probabilities: &'static [f64],
    confidence: f64,
}

// Generated with laya_mlx.common.build_sequence and ONNX Runtime 1.24.4 CPU
// for checkpoint d9d003d543e63d6d3375c21d44624136bd1e0bad. The expected noul
// value records runtime parity; it is not an assertion of factual accuracy.
const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "choice_en",
        text: "The customer wrote: I love the product and the support team was excellent.",
        kind: LayaQuestionType::Choice,
        instructions: "Classify the sentiment of the customer message.",
        criteria: &["positive", "neutral", "negative"],
        input_ids: &[
            2, 6241, 2872, 235292, 51110, 4739, 573, 25627, 576, 573, 6650, 3969, 235265, 1, 4,
            6222, 4, 17120, 4, 8322, 1, 714, 6650, 6559, 235292, 590, 2182, 573, 3225, 578, 573,
            2676, 2970, 729, 7943, 235265, 1,
        ],
        marker_pos: &[14, 16, 18],
        probabilities: &[0.8819, 0.1177, 0.0004],
        confidence: 0.6671,
    },
    Fixture {
        name: "choice_de",
        text: "Die Lieferung war kaputt und der Kundenservice hat nicht geantwortet.",
        kind: LayaQuestionType::Choice,
        instructions: "Classify the sentiment of the customer message.",
        criteria: &["positive", "neutral", "negative"],
        input_ids: &[
            2, 6241, 2872, 235292, 51110, 4739, 573, 25627, 576, 573, 6650, 3969, 235265, 1, 4,
            6222, 4, 17120, 4, 8322, 1, 3804, 134870, 2036, 12012, 22931, 930, 1188, 64709, 201033,
            3954, 3561, 1465, 741, 137371, 235265, 1,
        ],
        marker_pos: &[14, 16, 18],
        probabilities: &[0.0002, 0.0127, 0.9872],
        confidence: 0.9368,
    },
    Fixture {
        name: "score",
        text: "The customer wrote: I love the product and the support team was excellent.",
        kind: LayaQuestionType::Score,
        instructions: "Rate how satisfied the customer is.",
        criteria: &[
            "very dissatisfied",
            "dissatisfied",
            "neutral",
            "satisfied",
            "very satisfied",
        ],
        input_ids: &[
            2, 7716, 2872, 235292, 19129, 1368, 16202, 573, 6650, 603, 235265, 1, 4, 2403, 235248,
            235276, 235292, 1508, 103361, 4, 2403, 235248, 235274, 235292, 103361, 4, 2403, 235248,
            235284, 235292, 17120, 4, 2403, 235248, 235304, 235292, 16202, 4, 2403, 235248, 235310,
            235292, 1508, 16202, 1, 714, 6650, 6559, 235292, 590, 2182, 573, 3225, 578, 573, 2676,
            2970, 729, 7943, 235265, 1,
        ],
        marker_pos: &[12, 19, 25, 31, 37],
        probabilities: &[0.0527, 0.2706, 0.0436, 0.0131, 0.62],
        confidence: 0.3796,
    },
    Fixture {
        name: "noul",
        text: "Paris is the capital of France.",
        kind: LayaQuestionType::Noul,
        instructions: "The text states that Paris is the capital of France.",
        criteria: &[],
        input_ids: &[
            2, 552, 6311, 2872, 235292, 714, 2793, 6246, 674, 7127, 603, 573, 6037, 576, 6081,
            235265, 1, 4, 1566, 235292, 793, 235269, 573, 6218, 1721, 780, 3385, 4, 1382, 235292,
            7778, 235269, 573, 6218, 12723, 1, 7127, 603, 573, 6037, 576, 6081, 235265, 1,
        ],
        marker_pos: &[17, 27],
        probabilities: &[0.718, 0.282],
        confidence: 0.718,
    },
];

fn assets() -> (PathBuf, Tokenizer, LayaConfig) {
    let root = PathBuf::from(std::env::var_os("LAYA_MODEL_DIR").expect(
        "Set LAYA_MODEL_DIR to a mizchi/laya-multilingual-onnx snapshot containing model.onnx, tokenizer/tokenizer.json, and rl_agent_config.json",
    ));
    let mut tokenizer = Tokenizer::from_file(root.join("tokenizer/tokenizer.json"))
        .expect("load the checkpoint tokenizer");
    tokenizer.with_padding(None);
    tokenizer.with_truncation(None).unwrap();
    let config: LayaConfig = serde_json::from_slice(
        &fs::read(root.join("rl_agent_config.json")).expect("read Laya config"),
    )
    .expect("parse Laya config");
    config.validate().unwrap();
    (root, tokenizer, config)
}

fn options(fixture: &Fixture) -> LayaOptions {
    LayaOptions {
        question_type: fixture.kind,
        instructions: fixture.instructions.into(),
        criteria: fixture
            .criteria
            .iter()
            .map(|value| (*value).into())
            .collect(),
    }
}

fn synthetic_tokenizer() -> Tokenizer {
    let vocabulary = [
        "<unk>",
        "<bos>",
        "<eos>",
        "<mask>",
        "choice",
        "question:",
        "ask",
        "left",
        "right",
        "state",
        "option",
    ]
    .into_iter()
    .enumerate()
    .map(|(id, token)| (token.to_string(), id as u32))
    .collect();
    let model = tokenizers::models::wordlevel::WordLevel::builder()
        .vocab(vocabulary)
        .unk_token("<unk>".into())
        .build()
        .unwrap();
    let mut tokenizer = Tokenizer::new(model);
    tokenizer.with_pre_tokenizer(Some(
        tokenizers::pre_tokenizers::whitespace::WhitespaceSplit,
    ));
    tokenizer
}

fn choice(criteria: &[&str]) -> LayaOptions {
    LayaOptions {
        question_type: LayaQuestionType::Choice,
        instructions: "ask".into(),
        criteria: criteria.iter().map(|value| (*value).into()).collect(),
    }
}

#[test]
fn literal_mask_tokens_cannot_create_extra_option_markers() {
    let tokenizer = synthetic_tokenizer();
    let mut options = choice(&["left <mask>", "right <mask>"]);
    options.instructions = "ask <mask>".into();
    let prepared = prepare_laya(
        &tokenizer,
        "state <mask> state",
        &options,
        &LayaConfig::default(),
    )
    .unwrap();
    assert_eq!(prepared.input_ids, [1, 4, 5, 6, 2, 3, 7, 3, 8, 2, 9, 9, 2]);
    assert_eq!(prepared.marker_pos, [5, 7]);
}

#[test]
fn each_option_is_limited_to_48_tokens_after_its_marker() {
    let long_option = vec!["option"; 60].join(" ");
    let prepared = prepare_laya(
        &synthetic_tokenizer(),
        "state",
        &choice(&[&long_option, "right"]),
        &LayaConfig::default(),
    )
    .unwrap();
    assert_eq!(prepared.marker_pos, [5, 54]);
    assert_eq!(&prepared.input_ids[6..54], &[10; 48]);
    assert_eq!(&prepared.input_ids[54..], &[3, 8, 2, 9, 2]);
}

#[test]
fn long_state_is_right_truncated_with_a_final_separator() {
    let config = LayaConfig {
        max_len: 20,
        head_max_len: 16,
        ..LayaConfig::default()
    };
    let prepared = prepare_laya(
        &synthetic_tokenizer(),
        &vec!["state"; 100].join(" "),
        &choice(&["left", "right"]),
        &config,
    )
    .unwrap();
    assert_eq!(prepared.input_ids.len(), 20);
    assert_eq!(prepared.marker_pos, [5, 7]);
    assert_eq!(&prepared.input_ids[10..19], &[9; 9]);
    assert_eq!(prepared.input_ids.last(), Some(&2));
}

#[test]
fn options_whose_markers_exceed_sequence_capacity_are_rejected() {
    let config = LayaConfig {
        max_len: 32,
        head_max_len: 16,
        ..LayaConfig::default()
    };
    let mut options = choice(&["left"]);
    options.criteria = (0..16)
        .map(|index| format!("option {index} left right"))
        .collect();
    let error = prepare_laya(&synthetic_tokenizer(), "state", &options, &config).unwrap_err();
    assert!(error.to_string().contains("too many criteria"), "{error}");
}

#[test]
#[ignore = "requires LAYA_MODEL_DIR pointing to the published checkpoint"]
fn laya_prompt_matches_reference_token_ids_and_markers() {
    let (_, tokenizer, config) = assets();
    for fixture in FIXTURES {
        let prepared = prepare_laya(&tokenizer, fixture.text, &options(fixture), &config)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
        assert_eq!(
            prepared.input_ids, fixture.input_ids,
            "{} token IDs",
            fixture.name
        );
        assert_eq!(
            prepared.marker_pos, fixture.marker_pos,
            "{} option markers",
            fixture.name
        );
    }
}

#[test]
#[ignore = "requires LAYA_MODEL_DIR and loads the 647 MB Laya ONNX checkpoint"]
fn laya_choice_score_and_noul_match_reference_inference() {
    let (root, tokenizer, config) = assets();
    let mut session = configured_session_builder()
        .unwrap()
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .unwrap()
        .with_intra_threads(4)
        .unwrap()
        .commit_from_file(root.join("model.onnx"))
        .expect("load Laya with the configured FlowLike ONNX runtime");
    for fixture in FIXTURES {
        let result = infer_laya(
            &mut session,
            &tokenizer,
            fixture.text,
            &options(fixture),
            &config,
        )
        .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
        assert_eq!(result.question_type, fixture.kind);
        assert_eq!(result.input_tokens, fixture.input_ids.len());
        assert_eq!(result.probabilities.len(), fixture.probabilities.len());
        // Float16 execution can vary slightly between CPU architectures/providers.
        for (actual, expected) in result.probabilities.iter().zip(fixture.probabilities) {
            assert!(
                (actual.probability - expected).abs() < 0.02,
                "{} probability for {}: {} vs {expected}",
                fixture.name,
                actual.label,
                actual.probability
            );
        }
        assert!(
            (result.confidence - fixture.confidence).abs() < 0.02,
            "{} confidence",
            fixture.name
        );
        assert!(
            (result.act_probability - 1.0).abs() < 0.001,
            "{} action probability",
            fixture.name
        );
        match fixture.kind {
            LayaQuestionType::Choice => {
                let expected = if fixture.name == "choice_en" {
                    "positive"
                } else {
                    "negative"
                };
                assert_eq!(result.choice.as_deref(), Some(expected));
                assert!(result.score.is_none() && result.noul.is_none());
                assert_eq!(
                    result
                        .probabilities
                        .iter()
                        .map(|value| value.label.as_str())
                        .collect::<Vec<_>>(),
                    fixture.criteria
                );
            }
            LayaQuestionType::Score => {
                assert!((result.score.unwrap() - 2.8770).abs() < 0.04);
                assert!(result.choice.is_none() && result.noul.is_none());
                assert_eq!(
                    result
                        .probabilities
                        .iter()
                        .map(|value| value.label.as_str())
                        .collect::<Vec<_>>(),
                    ["0", "1", "2", "3", "4"]
                );
            }
            LayaQuestionType::Noul => {
                assert!((result.noul.unwrap() - 0.2820).abs() < 0.02);
                assert!(result.choice.is_none() && result.score.is_none());
                assert_eq!(
                    result
                        .probabilities
                        .iter()
                        .map(|value| value.label.as_str())
                        .collect::<Vec<_>>(),
                    ["false", "true"]
                );
            }
        }
    }
}
