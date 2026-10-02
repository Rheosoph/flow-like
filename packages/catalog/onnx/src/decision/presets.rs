//! Pinned classification exports of the Fastino checkpoints.
use super::super::model_cache::{DECISION_MODELS, ModelSpec};
use flow_like_types::Result;

pub(super) struct Preset {
    pub assets: Vec<ModelSpec>,
    pub layout: PresetLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PresetLayout {
    Combined,
    Split,
    Bundle,
}

pub(super) fn preset(model: &str) -> Result<Option<Preset>> {
    if let Some(preset) = cdn_preset(model)? { return Ok(Some(preset)); }
    let (repo, revision, files): (&str, &str, &[(&str, &str, u64, &str)]) = match model {
        "fastino/GLiNER2.5-Decide" => (
            "nishparadox/gliner2.5-decide-onnx",
            "bbbcdb01c406b4f9a44b3a8102c96edb91c1f47d",
            &[
                (
                    "weights",
                    "model.onnx",
                    1_745_911_616,
                    "dce49c567af4415889b36b6adad35ecae049cca6a81f55c6cfa34825111b0dcf",
                ),
                (
                    "tokenizer",
                    "tokenizer.json",
                    8_333_952,
                    "3ad87d9ffe669147063e70850927dd2da90249e2acc5c8527f1eb65df467bcc8",
                ),
            ],
        ),
        "fastino/gliner2.5-small-v1" => (
            "codesoda/gliner2-onnx",
            "27310cd26099a387b9936a1e13b03d6a0700baf2",
            &[
                (
                    "encoder",
                    "gliner2.5-small-v1/encoder.onnx",
                    283_500_531,
                    "1bbc3846c2bdab94528377754ec492e63eceb38b5599c1196fac97a9f16fe5db",
                ),
                (
                    "classifier",
                    "gliner2.5-small-v1/classifier.onnx",
                    1_186_690,
                    "2fa5e9fe06ef2b3252bb2af01ed918f83e63f4ba28f36b158770b50b5aec405a",
                ),
                (
                    "tokenizer",
                    "gliner2.5-small-v1/tokenizer.json",
                    8_341_713,
                    "cbc8ae6037812709c9c26f2a160f8dc48b0440bcb79c8141804259ae2d6adac3",
                ),
            ],
        ),
        "fastino/gliner2.5-base-v1" => (
            "codesoda/gliner2-onnx",
            "27310cd26099a387b9936a1e13b03d6a0700baf2",
            &[
                (
                    "encoder",
                    "gliner2.5-base-v1/encoder.onnx",
                    735_961_685,
                    "04f601e1a63fd829880274e4b61d6ba1d1170c4342cdc956e27aaf50e8b01957",
                ),
                (
                    "classifier",
                    "gliner2.5-base-v1/classifier.onnx",
                    4_731_781,
                    "045e94e66de4d8d4d9d267b18bc864328b7661879b7ec149b3e353a94ee1b3ca",
                ),
                (
                    "tokenizer",
                    "gliner2.5-base-v1/tokenizer.json",
                    8_341_713,
                    "cbc8ae6037812709c9c26f2a160f8dc48b0440bcb79c8141804259ae2d6adac3",
                ),
            ],
        ),
        "fastino/gliner2.5-multi-v1" => (
            "codesoda/gliner2-onnx",
            "27310cd26099a387b9936a1e13b03d6a0700baf2",
            &[
                (
                    "encoder",
                    "gliner2.5-multi-v1/encoder.onnx",
                    1_111_055_957,
                    "345c9f767ab4ea91ac675ce7d1ad53a12941b3609539853883556e9b754a9b7a",
                ),
                (
                    "classifier",
                    "gliner2.5-multi-v1/classifier.onnx",
                    4_731_781,
                    "b29bf1647f9e40f54a74c8048c1cc6de96a5f6acdf4ac9a293babf99464752d3",
                ),
                (
                    "tokenizer",
                    "gliner2.5-multi-v1/tokenizer.json",
                    16_035_853,
                    "c62446df87ae18ec98b133f8f84fc449a07cc89bbf8ef192a4cb5f9c53777a7a",
                ),
            ],
        ),
        _ => return Ok(None),
    };
    let assets = files
        .iter()
        .map(|(role, file, size, hash)| {
            ModelSpec::new(
                &DECISION_MODELS,
                role,
                *size,
                &format!("https://huggingface.co/{repo}/resolve/{revision}/{file}"),
                hash,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(Preset {
        layout: if files.len() == 3 { PresetLayout::Split } else { PresetLayout::Combined },
        assets,
    }))
}

fn cdn_preset(model: &str) -> Result<Option<Preset>> {
    let (base, files): (&str, &[(&str, &str, u64, &str)]) = match model {
        "fastino/GLiNER2.5-multi-Decide" => ("https://cdn.flow-like.com/models/decision/gliner2.5-multi-decide/a35a0cd3b7a0f00f2effc576f454cd48fa98aa5f/fp32-v1", &[
            ("weights", "model.onnx", 1_068_422, "c2cf6b9b80f3524d4c99e5fef2428f24e97d24d98f4e8224707d229fa38c0a36"),
            ("external-data", "model.onnx_data", 1_114_877_952, "9bbe122d2c0c42b6e6d8baf03b028ef6e4d3672c5757b538c30f556ce8980765"),
            ("tokenizer", "tokenizer.json", 16_020_605, "b107e2e5998e7429491aaae7807d8b0815ff3e06fcb5ba95bc6a35c9b93185c5"),
            ("config", "decision_config.json", 374, "33fab8719037e2593bb24a86aae6071e9184554edec2874aad53f389eb4afc30"),
        ]),
        _ => return Ok(None),
    };
    let assets = files.iter().map(|(role, file, size, hash)| {
        ModelSpec::new(&DECISION_MODELS, role, *size, &format!("{base}/{file}"), hash)
    }).collect::<Result<Vec<_>>>()?;
    Ok(Some(Preset { assets, layout: PresetLayout::Bundle }))
}
