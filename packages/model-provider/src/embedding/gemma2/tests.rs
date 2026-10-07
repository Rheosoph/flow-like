use std::sync::Arc;

use image::{DynamicImage, Rgb, RgbImage};

use super::*;
use crate::embedding::interface::VideoFrame;

fn image_fixture(phase: u32) -> Arc<DynamicImage> {
    Arc::new(DynamicImage::ImageRgb8(RgbImage::from_fn(
        80,
        64,
        |x, y| {
            Rgb([
                ((x * 3 + y + phase) % 256) as u8,
                ((y * 4 + phase) % 256) as u8,
                ((x + y * 2 + phase) % 256) as u8,
            ])
        },
    )))
}

fn audio_fixture() -> AudioInput {
    let samples: Vec<f32> = (0..16_000)
        .map(|i| {
            // Generate the fixture in f64 and round each sample once, as NumPy does.
            (0.2 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 16_000.0).sin()
                + 0.05 * (2.0 * std::f64::consts::PI * 1379.0 * i as f64 / 16_000.0).cos())
                as f32
        })
        .collect();
    AudioInput {
        samples: Arc::from(samples),
        sample_rate: 16_000,
        channels: 1,
    }
}

fn video_fixture() -> VideoInput {
    VideoInput {
        frames: vec![
            VideoFrame {
                timestamp_ms: 0,
                image: image_fixture(0),
            },
            VideoFrame {
                timestamp_ms: 1000,
                image: image_fixture(51),
            },
        ],
        duration_ms: 2000,
        audio: None,
    }
}

fn fixtures() -> Vec<EmbeddingInput> {
    vec![
        EmbeddingInput::text("A red fox jumps over a log."),
        EmbeddingInput {
            parts: vec![EmbeddingPart::Image(image_fixture(0))],
            title: None,
        },
        EmbeddingInput {
            parts: vec![EmbeddingPart::Audio(audio_fixture())],
            title: None,
        },
        EmbeddingInput {
            parts: vec![EmbeddingPart::Video(video_fixture())],
            title: None,
        },
        EmbeddingInput {
            parts: vec![
                EmbeddingPart::Text("A colorful scene ".into()),
                EmbeddingPart::Image(image_fixture(0)),
                EmbeddingPart::Text(" with sound ".into()),
                EmbeddingPart::Audio(audio_fixture()),
                EmbeddingPart::Video(video_fixture()),
            ],
            title: None,
        },
        EmbeddingInput {
            parts: vec![
                EmbeddingPart::Text("A colorful scene ".into()),
                EmbeddingPart::Image(image_fixture(0)),
            ],
            title: None,
        },
        EmbeddingInput {
            parts: vec![
                EmbeddingPart::Text("A recorded sound ".into()),
                EmbeddingPart::Audio(audio_fixture()),
            ],
            title: None,
        },
        EmbeddingInput {
            parts: vec![
                EmbeddingPart::Text("A colorful video ".into()),
                EmbeddingPart::Video(video_fixture()),
            ],
            title: None,
        },
        EmbeddingInput {
            parts: vec![EmbeddingPart::Video(VideoInput {
                audio: Some(audio_fixture()),
                ..video_fixture()
            })],
            title: None,
        },
    ]
}

#[test]
fn timestamp_sampling_keeps_timeline_order_and_uniform_endpoints() {
    let image = image_fixture(0);
    let video = VideoInput {
        frames: (0..100)
            .map(|i| VideoFrame {
                timestamp_ms: i * 1000,
                image: image.clone(),
            })
            .collect(),
        duration_ms: 100_000,
        audio: None,
    };
    let sampled = sample_video(&video, 4).unwrap();
    assert_eq!(sampled, [0, 33, 66, 99]);
    assert_eq!(sample_video(&video, 1).unwrap(), [0]);
    let sparse = VideoInput {
        frames: vec![video.frames[0].clone(), video.frames[2].clone()],
        duration_ms: 4000,
        audio: None,
    };
    assert_eq!(sample_video(&sparse, 32).unwrap(), [0, 0, 1, 1]);
}

#[test]
fn retrieval_and_other_tasks_use_distinct_reference_prompts() {
    assert_eq!(
        purpose_prefix(EmbeddingPurpose::Query, None),
        "task: search result | query: "
    );
    assert_eq!(
        purpose_prefix(EmbeddingPurpose::Document, Some("Birds")),
        "title: Birds | text: "
    );
    assert_eq!(
        purpose_prefix(EmbeddingPurpose::CodeQuery, None),
        "task: code retrieval | query: "
    );
}

#[test]
fn empty_text_keeps_its_task_and_titles_must_be_encoded() {
    let empty = EmbeddingInput::text("");
    assert_eq!(
        input_prefix(&empty, EmbeddingPurpose::Query).unwrap(),
        "task: search result | query: "
    );
    assert_eq!(
        input_prefix(&empty, EmbeddingPurpose::Document).unwrap(),
        "title: none | text: "
    );
    let media = EmbeddingInput {
        parts: vec![EmbeddingPart::Image(image_fixture(0))],
        title: None,
    };
    assert_eq!(
        input_prefix(&media, EmbeddingPurpose::Document).unwrap(),
        ""
    );
    let mut titled = empty;
    titled.title = Some("Birds".into());
    assert_eq!(
        input_prefix(&titled, EmbeddingPurpose::Document).unwrap(),
        "title: Birds | text: "
    );
    for purpose in [
        EmbeddingPurpose::Query,
        EmbeddingPurpose::Similarity,
        EmbeddingPurpose::Classification,
        EmbeddingPurpose::Clustering,
        EmbeddingPurpose::CodeQuery,
    ] {
        assert!(input_prefix(&titled, purpose).is_err(), "{purpose:?}");
    }
    let titled_media = EmbeddingInput {
        title: Some("Birds".into()),
        ..media
    };
    assert!(input_prefix(&titled_media, EmbeddingPurpose::Document).is_err());
}

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    let dot = left
        .iter()
        .zip(right)
        .map(|(a, b)| *a as f64 * *b as f64)
        .sum::<f64>();
    let norm = |values: &[f32]| {
        values
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    dot / (norm(left) * norm(right))
}

#[test]
#[ignore = "requires the pinned ONNX pack in FLOW_LIKE_GEMMA2_ASSETS"]
fn multimodal_embeddings_match_reference() {
    let directory = PathBuf::from(
        std::env::var_os("FLOW_LIKE_GEMMA2_ASSETS").expect("FLOW_LIKE_GEMMA2_ASSETS"),
    );
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/gemma2/reference.json"
    ))
    .unwrap();
    let mut model = Gemma2Embedding::load(&directory, Gemma2Options::default()).unwrap();
    for purpose in [EmbeddingPurpose::Query, EmbeddingPurpose::Document] {
        let expected = model
            .tokenizer
            .encode(purpose_prefix(purpose, None), true)
            .unwrap();
        assert_eq!(
            model.input_ids(&EmbeddingInput::text(""), purpose).unwrap(),
            expected
                .get_ids()
                .iter()
                .copied()
                .map(i64::from)
                .collect::<Vec<_>>()
        );
    }
    assert!(
        model
            .input_ids(
                &EmbeddingInput {
                    parts: vec![EmbeddingPart::Image(image_fixture(0))],
                    title: Some("Birds".into())
                },
                EmbeddingPurpose::Document
            )
            .is_err()
    );
    let inputs = fixtures();
    let vectors = model.embed(&inputs, EmbeddingPurpose::Document).unwrap();
    for (index, vector) in vectors.iter().enumerate() {
        let expected: Vec<f32> =
            serde_json::from_value(reference["embeddings"][index].clone()).unwrap();
        assert_eq!(vector.len(), DIMENSIONS);
        let similarity = cosine(vector, &expected);
        eprintln!("modality fixture {index}: reference cosine {similarity:.9}");
        assert!(
            similarity > 0.999999,
            "fixture {index}: cosine {similarity}"
        );
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
    let audio = audio::preprocess(&audio_fixture()).unwrap();
    let audio_max_error = sampled_error(&audio.values, &reference["audio_features"]);
    eprintln!("audio preprocessing maximum absolute error: {audio_max_error}");
    assert!(audio_max_error < 1e-4);
    let vision = vision::preprocess(&image_fixture(0), 280).unwrap();
    let pixel_max_error = sampled_error(&vision.pixels, &reference["image_pixels"]);
    eprintln!("image preprocessing maximum absolute error: {pixel_max_error}");
    assert!(pixel_max_error < 1e-6);
    // Ensure request reordering, repeat calls and MRL do not change the embedding space.
    let repeated = model
        .embed(
            &[inputs[1].clone(), inputs[0].clone()],
            EmbeddingPurpose::Document,
        )
        .unwrap();
    assert!(cosine(&repeated[0], &vectors[1]) > 0.999999);
    assert!(cosine(&repeated[1], &vectors[0]) > 0.999999);
    model.options.dimensions = 128;
    let reduced = model
        .embed(&inputs[..1], EmbeddingPurpose::Document)
        .unwrap();
    assert_eq!(reduced[0].len(), 128);
    assert!(cosine(&reduced[0], &vectors[0][..128]) > 0.999999);
    assert!(
        model
            .embed(&[EmbeddingInput::text(IMAGE)], EmbeddingPurpose::Document)
            .is_err()
    );
    model.options.max_tokens = 3;
    assert!(
        model
            .embed(&inputs[..1], EmbeddingPurpose::Document)
            .is_err()
    );
    // Budget validation must stop before touching the media encoder.
    assert!(
        model
            .input_ids(&inputs[1], EmbeddingPurpose::Document)
            .is_err()
    );
}

fn sampled_error(actual: &[f32], reference: &serde_json::Value) -> f32 {
    assert_eq!(actual.len(), reference["length"].as_u64().unwrap() as usize);
    let indices: Vec<usize> = serde_json::from_value(reference["indices"].clone()).unwrap();
    let values: Vec<f32> = serde_json::from_value(reference["values"].clone()).unwrap();
    indices
        .iter()
        .zip(values)
        .map(|(index, expected)| (actual[*index] - expected).abs())
        .fold(0.0_f32, f32::max)
}
