//! Unit tests for ONNX catalog nodes
//!
//! These tests verify node structure, metadata, and type correctness.
//! Model-dependent tests require actual ONNX model files and are marked with #[ignore].
//!
//! Run unit tests: cargo test --package flow-like-catalog-onnx --test node_tests
//! Run all tests: cargo test --package flow-like-catalog-onnx --test node_tests -- --include-ignored

extern crate flow_like_runtime as flow_like;

use flow_like::flow::node::NodeLogic;
use flow_like_catalog_onnx::{
    audio::{AudioData, SpeechSegment, TranscriptionSegment, VadResult},
    depth::{DepthMap, DepthProvider},
    face::{DetectedFace, FaceEmbedding, FaceLandmarks, LandmarkType},
    ocr::{OcrRegion, OcrResult, RecognizedText, TextRegion},
};

// ============================================================================
// Audio Type Tests
// ============================================================================

mod audio_types {
    use super::*;

    #[test]
    fn audio_data_creation() {
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let audio = AudioData::new(16000, 1, samples.clone());

        assert_eq!(audio.sample_rate, 16000);
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.samples, samples);
        assert!((audio.duration_secs - 0.0003125).abs() < 0.0001);
    }

    #[test]
    fn audio_data_mono_passthrough() {
        let audio = AudioData::new(16000, 1, vec![0.5, 0.5]);
        let mono = audio.to_mono();
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.samples, audio.samples);
    }

    #[test]
    fn audio_data_stereo_to_mono() {
        let audio = AudioData::new(16000, 2, vec![0.0, 1.0, 0.5, 0.5]);
        let mono = audio.to_mono();
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.samples.len(), 2);
        assert!((mono.samples[0] - 0.5).abs() < 0.001);
        assert!((mono.samples[1] - 0.5).abs() < 0.001);
    }

    #[test]
    fn audio_data_resample_passthrough() {
        let audio = AudioData::new(16000, 1, vec![0.5, 0.5, 0.5]);
        let resampled = audio.resample(16000);
        assert_eq!(resampled.sample_rate, 16000);
        assert_eq!(resampled.samples.len(), 3);
    }

    #[test]
    fn audio_data_resample_upsample() {
        let audio = AudioData::new(8000, 1, vec![0.0, 1.0, 0.0, 1.0]);
        let resampled = audio.resample(16000);
        assert_eq!(resampled.sample_rate, 16000);
        assert!(resampled.samples.len() >= 7);
    }

    #[test]
    fn speech_segment_serialize() {
        let segment = SpeechSegment {
            start: 0.5,
            end: 1.5,
            confidence: 0.95,
        };
        let json = serde_json::to_string(&segment).unwrap();
        assert!(json.contains("0.5"));
        assert!(json.contains("1.5"));
        assert!(json.contains("0.95"));
    }

    #[test]
    fn transcription_segment_serialize() {
        let segment = TranscriptionSegment {
            text: "hello".to_string(),
            start: 0.0,
            end: 0.5,
            confidence: 0.9,
        };
        let json = serde_json::to_string(&segment).unwrap();
        assert!(json.contains("hello"));
    }

    #[test]
    fn vad_result_serialize() {
        let result = VadResult {
            segments: vec![SpeechSegment {
                start: 0.0,
                end: 1.0,
                confidence: 0.95,
            }],
            probabilities: vec![0.1, 0.9, 0.95],
            frame_duration: 0.032,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("segments"));
        assert!(json.contains("probabilities"));
    }
}

// ============================================================================
// Depth Type Tests
// ============================================================================

mod depth_types {
    use super::*;

    #[test]
    fn depth_map_get_depth() {
        let depth_map = DepthMap {
            values: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
            width: 3,
            height: 2,
            min_depth: 0.0,
            max_depth: 1.0,
        };

        assert_eq!(depth_map.get_depth(0, 0), Some(0.1));
        assert_eq!(depth_map.get_depth(2, 0), Some(0.3));
        assert_eq!(depth_map.get_depth(0, 1), Some(0.4));
        assert_eq!(depth_map.get_depth(2, 1), Some(0.6));
        assert_eq!(depth_map.get_depth(3, 0), None);
        assert_eq!(depth_map.get_depth(0, 2), None);
    }

    #[test]
    fn depth_provider_default() {
        let provider = DepthProvider::default();
        match provider {
            DepthProvider::MiDaSLike => {}
            _ => panic!("Expected MiDaSLike as default"),
        }
    }

    #[test]
    fn depth_provider_serialize() {
        let provider = DepthProvider::DepthAnythingLike;
        let json = serde_json::to_string(&provider).unwrap();
        assert!(json.contains("DepthAnythingLike"));
    }

    #[test]
    fn depth_map_serialize() {
        let depth_map = DepthMap {
            values: vec![0.5],
            width: 1,
            height: 1,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let json = serde_json::to_string(&depth_map).unwrap();
        let deserialized: DepthMap = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.width, 1);
        assert_eq!(deserialized.height, 1);
    }
}

// ============================================================================
// Face Type Tests
// ============================================================================

mod face_types {
    use super::*;

    #[test]
    fn face_embedding_cosine_similarity_identical() {
        let emb = FaceEmbedding {
            embedding: vec![1.0, 0.0, 0.0],
            dimension: 3,
        };
        let sim = emb.cosine_similarity(&emb);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn face_embedding_cosine_similarity_orthogonal() {
        let emb1 = FaceEmbedding {
            embedding: vec![1.0, 0.0, 0.0],
            dimension: 3,
        };
        let emb2 = FaceEmbedding {
            embedding: vec![0.0, 1.0, 0.0],
            dimension: 3,
        };
        let sim = emb1.cosine_similarity(&emb2);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn face_embedding_cosine_similarity_opposite() {
        let emb1 = FaceEmbedding {
            embedding: vec![1.0, 0.0, 0.0],
            dimension: 3,
        };
        let emb2 = FaceEmbedding {
            embedding: vec![-1.0, 0.0, 0.0],
            dimension: 3,
        };
        let sim = emb1.cosine_similarity(&emb2);
        assert!((sim + 1.0).abs() < 0.001);
    }

    #[test]
    fn face_embedding_euclidean_distance_same() {
        let emb = FaceEmbedding {
            embedding: vec![1.0, 2.0, 3.0],
            dimension: 3,
        };
        let dist = emb.euclidean_distance(&emb);
        assert!(dist.abs() < 0.001);
    }

    #[test]
    fn face_embedding_euclidean_distance_unit() {
        let emb1 = FaceEmbedding {
            embedding: vec![0.0, 0.0, 0.0],
            dimension: 3,
        };
        let emb2 = FaceEmbedding {
            embedding: vec![1.0, 0.0, 0.0],
            dimension: 3,
        };
        let dist = emb1.euclidean_distance(&emb2);
        assert!((dist - 1.0).abs() < 0.001);
    }

    #[test]
    fn face_embedding_dimension_mismatch() {
        let emb1 = FaceEmbedding {
            embedding: vec![1.0, 0.0],
            dimension: 2,
        };
        let emb2 = FaceEmbedding {
            embedding: vec![1.0, 0.0, 0.0],
            dimension: 3,
        };
        let sim = emb1.cosine_similarity(&emb2);
        assert_eq!(sim, 0.0);
        let dist = emb1.euclidean_distance(&emb2);
        assert_eq!(dist, f32::MAX);
    }

    #[test]
    fn detected_face_serialize() {
        let face = DetectedFace {
            bbox: [10.0, 20.0, 100.0, 100.0],
            confidence: 0.98,
            landmarks: Some(FaceLandmarks {
                points: vec![
                    [30.0, 40.0],
                    [70.0, 40.0],
                    [50.0, 60.0],
                    [35.0, 80.0],
                    [65.0, 80.0],
                ],
                landmark_type: LandmarkType::FivePoint,
            }),
        };
        let json = serde_json::to_string(&face).unwrap();
        let deserialized: DetectedFace = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.confidence, 0.98);
        assert!(deserialized.landmarks.is_some());
    }

    #[test]
    fn landmark_type_default() {
        let lt = LandmarkType::default();
        match lt {
            LandmarkType::FivePoint => {}
            _ => panic!("Expected FivePoint as default"),
        }
    }
}

// ============================================================================
// OCR Type Tests
// ============================================================================

mod ocr_types {
    use super::*;

    #[test]
    fn text_region_serialize() {
        let region = TextRegion {
            bbox: [10.0, 20.0, 100.0, 30.0],
            polygon: [[10.0, 20.0], [110.0, 20.0], [110.0, 50.0], [10.0, 50.0]],
            confidence: 0.95,
        };
        let json = serde_json::to_string(&region).unwrap();
        let deserialized: TextRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.confidence, 0.95);
    }

    #[test]
    fn recognized_text_serialize() {
        let text = RecognizedText {
            text: "Hello World".to_string(),
            confidence: 0.92,
            char_confidences: vec![0.95, 0.90, 0.88, 0.99],
        };
        let json = serde_json::to_string(&text).unwrap();
        assert!(json.contains("Hello World"));
    }

    #[test]
    fn ocr_result_full_text() {
        let result = OcrResult {
            regions: vec![
                OcrRegion {
                    region: TextRegion {
                        bbox: [0.0, 0.0, 50.0, 20.0],
                        polygon: [[0.0, 0.0], [50.0, 0.0], [50.0, 20.0], [0.0, 20.0]],
                        confidence: 0.9,
                    },
                    text: RecognizedText {
                        text: "Hello".to_string(),
                        confidence: 0.9,
                        char_confidences: vec![],
                    },
                },
                OcrRegion {
                    region: TextRegion {
                        bbox: [60.0, 0.0, 50.0, 20.0],
                        polygon: [[60.0, 0.0], [110.0, 0.0], [110.0, 20.0], [60.0, 20.0]],
                        confidence: 0.85,
                    },
                    text: RecognizedText {
                        text: "World".to_string(),
                        confidence: 0.85,
                        char_confidences: vec![],
                    },
                },
            ],
            full_text: "Hello World".to_string(),
        };
        assert_eq!(result.regions.len(), 2);
        assert_eq!(result.full_text, "Hello World");
    }
}

// ============================================================================
// Node Metadata Tests
// ============================================================================

mod node_metadata {
    use super::*;
    use flow_like_catalog_onnx::{
        audio::{LoadAudioNode, ResampleAudioNode, TrimAudioNode, VoiceActivityDetectionNode},
        batch::BatchImageInferenceNode,
        depth::{DepthColorizeNode, DepthEstimationNode, DepthToPointCloudNode},
        face::{CompareFacesNode, CropFacesNode, FaceDetectionNode, FaceEmbeddingNode},
        face_id::{AnalyzeFacesNode, LoadFaceAnalyzerNode, UnloadFaceAnalyzerNode},
        ocr::{CropTextRegionsNode, TextDetectionNode, TextRecognitionNode},
    };

    fn assert_node_has_exec_pins(node: &flow_like::flow::node::Node) {
        assert!(
            node.pins
                .values()
                .any(|p| p.name.contains("exec") || p.name == "Input"),
            "Node should have execution pin"
        );
    }

    fn assert_node_has_description(node: &flow_like::flow::node::Node) {
        assert!(!node.description.is_empty(), "Node should have description");
    }

    fn pin<'a>(node: &'a flow_like::flow::node::Node, name: &str) -> &'a flow_like::flow::pin::Pin {
        node.pins
            .values()
            .find(|pin| pin.name == name)
            .unwrap_or_else(|| panic!("Node {} is missing pin {name}", node.name))
    }

    #[test]
    fn depth_estimation_node_metadata() {
        let node_logic = DepthEstimationNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Depth Estimation");
        assert!(node.description.contains("https://"));
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn depth_to_point_cloud_node_metadata() {
        let node_logic = DepthToPointCloudNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Depth to Point Cloud");
        assert_node_has_exec_pins(&node);
        assert_node_has_description(&node);
    }

    #[test]
    fn depth_colorize_node_metadata() {
        let node_logic = DepthColorizeNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Colorize Depth");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn face_detection_node_metadata() {
        let node_logic = FaceDetectionNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Face Detection");
        assert!(node.description.contains("https://"));
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn face_embedding_node_metadata() {
        let node_logic = FaceEmbeddingNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Face Embedding");
        assert!(node.description.contains("https://"));
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn compare_faces_node_metadata() {
        let node_logic = CompareFacesNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Compare Faces");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn crop_faces_node_metadata() {
        let node_logic = CropFacesNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Crop Faces");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn load_face_analyzer_node_metadata() {
        let node = LoadFaceAnalyzerNode::new().get_node();

        assert_eq!(node.name, "face_id_load_analyzer");
        assert_eq!(node.version, Some(2));
        assert_eq!(node.category, "AI/ML/ONNX/Face");
        assert_node_has_exec_pins(&node);
        assert!(pin(&node, "cache_dir").schema.is_some());
        assert!(pin(&node, "analyzer").schema.is_some());
        for name in [
            "detector_url",
            "detector_sha256",
            "embedder_url",
            "embedder_sha256",
            "gender_age_url",
            "gender_age_sha256",
        ] {
            assert!(pin(&node, name).default_value.is_some());
        }
        assert_eq!(
            pin(&node, "input_size").options.as_ref().unwrap().range,
            Some((32.0, 640.0))
        );
        assert_eq!(
            pin(&node, "score_threshold")
                .options
                .as_ref()
                .unwrap()
                .range,
            Some((0.25, 1.0))
        );
        assert_eq!(
            pin(&node, "iou_threshold").options.as_ref().unwrap().range,
            Some((0.0, 0.75))
        );
    }

    #[test]
    fn analyze_faces_node_metadata() {
        let node = AnalyzeFacesNode::new().get_node();

        assert_eq!(node.name, "face_id_analyze");
        assert_eq!(node.version, Some(2));
        assert_node_has_exec_pins(&node);
        assert_eq!(
            pin(&node, "max_faces").options.as_ref().unwrap().range,
            Some((1.0, 100.0))
        );
        let faces = pin(&node, "faces");
        assert_eq!(
            faces.data_type,
            flow_like::flow::variable::VariableType::Struct
        );
        assert_eq!(faces.value_type, flow_like::flow::pin::ValueType::Array);
        assert!(faces.schema.is_some());
    }

    #[test]
    fn unload_face_analyzer_node_metadata() {
        let node = UnloadFaceAnalyzerNode::new().get_node();

        assert_eq!(node.name, "face_id_unload_analyzer");
        assert_eq!(node.version, Some(1));
        assert_node_has_exec_pins(&node);
        assert!(pin(&node, "analyzer").schema.is_some());
        assert_eq!(
            pin(&node, "success").data_type,
            flow_like::flow::variable::VariableType::Boolean
        );
    }

    #[test]
    fn text_detection_node_metadata() {
        let node_logic = TextDetectionNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Text Detection");
        assert!(node.description.contains("https://"));
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn text_recognition_node_metadata() {
        let node_logic = TextRecognitionNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Text Recognition");
        assert!(node.description.contains("https://"));
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn crop_text_regions_node_metadata() {
        let node_logic = CropTextRegionsNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Crop Text Regions");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn load_audio_node_metadata() {
        let node_logic = LoadAudioNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Load Audio");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn vad_node_metadata() {
        let node_logic = VoiceActivityDetectionNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Voice Activity Detection");
        assert!(node.description.contains("https://"));
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn resample_audio_node_metadata() {
        let node_logic = ResampleAudioNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Resample Audio");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn trim_audio_node_metadata() {
        let node_logic = TrimAudioNode::default();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Trim Audio");
        assert_node_has_exec_pins(&node);
    }

    #[test]
    fn batch_image_inference_node_metadata() {
        let node_logic = BatchImageInferenceNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.friendly_name, "Batch Image Inference");
        assert_node_has_exec_pins(&node);
    }
}
