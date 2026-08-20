//! Integration tests for ONNX catalog nodes with actual models
//!
//! These tests download small ONNX models and run actual inference.
//! They are marked #[ignore] by default since they require network access and take time.
//!
//! Run integration tests:
//! ```sh
//! cargo test --package flow-like-catalog-onnx --test integration_tests --features execute -- --ignored
//! ```
//!
//! Run a specific test:
//! ```sh
//! cargo test --package flow-like-catalog-onnx --test integration_tests --features execute -- --ignored test_depth_midas
//! ```

use std::fs;
use std::io::Write;
use std::path::PathBuf;

const TEST_MODELS_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn test_data_dir() -> PathBuf {
    PathBuf::from(TEST_MODELS_DIR).join("tests").join("models")
}

fn ensure_test_dir() -> PathBuf {
    let dir = test_data_dir();
    fs::create_dir_all(&dir).expect("Failed to create test models directory");
    dir
}

#[cfg(feature = "execute")]
fn configured_session_builder() -> flow_like_model_provider::ml::ort::Result<
    flow_like_model_provider::ml::ort::session::builder::SessionBuilder,
> {
    flow_like_model_provider::ml::ort_runtime::configured_session_builder()
}

/// Download a file from URL if it doesn't exist locally
fn download_if_missing(url: &str, filename: &str) -> PathBuf {
    let dir = ensure_test_dir();
    let path = dir.join(filename);

    if path.exists() {
        println!("Model already exists: {}", path.display());
        return path;
    }

    println!("Downloading {} from {}...", filename, url);

    let response =
        reqwest::blocking::get(url).unwrap_or_else(|_| panic!("Failed to download {}", url));

    if !response.status().is_success() {
        panic!("Failed to download {}: HTTP {}", url, response.status());
    }

    let bytes = response.bytes().expect("Failed to read response body");

    let mut file = fs::File::create(&path).expect("Failed to create file");
    file.write_all(&bytes).expect("Failed to write file");

    println!("Downloaded {} ({} bytes)", filename, bytes.len());
    path
}

/// Create a simple test image (solid color)
#[allow(dead_code)]
fn create_test_image(width: u32, height: u32) -> image::DynamicImage {
    use image::{Rgb, RgbImage};

    let mut img = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            // Create a gradient pattern
            let r = (x * 255 / width) as u8;
            let g = (y * 255 / height) as u8;
            let b = 128u8;
            img.put_pixel(x, y, Rgb([r, g, b]));
        }
    }
    image::DynamicImage::ImageRgb8(img)
}

/// Create a test image with face-like features (for face detection testing)
fn create_face_test_image() -> image::DynamicImage {
    use image::{Rgb, RgbImage};

    let mut img = RgbImage::new(640, 480);

    // Fill with skin-tone-ish background
    for y in 0..480 {
        for x in 0..640 {
            img.put_pixel(x, y, Rgb([200, 180, 160]));
        }
    }

    // Draw a simple face-like oval in the center
    let cx = 320.0f32;
    let cy = 240.0f32;
    let rx = 80.0f32;
    let ry = 100.0f32;

    for y in 0..480 {
        for x in 0..640 {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            if dx * dx + dy * dy < 1.0 {
                img.put_pixel(x, y, Rgb([220, 190, 170]));
            }
        }
    }

    // Draw "eyes" (dark spots)
    for ey in [-30i32, -30] {
        for ex in [-25i32, 25] {
            let eye_cx = cx as i32 + ex;
            let eye_cy = cy as i32 + ey;
            for dy in -8..8 {
                for dx in -8..8 {
                    if dx * dx + dy * dy < 64 {
                        let px = (eye_cx + dx) as u32;
                        let py = (eye_cy + dy) as u32;
                        if px < 640 && py < 480 {
                            img.put_pixel(px, py, Rgb([50, 40, 30]));
                        }
                    }
                }
            }
        }
    }

    image::DynamicImage::ImageRgb8(img)
}

// ============================================================================
// Model URLs - Using small/quantized models where possible
// ============================================================================

// Depth estimation (large model - skip in CI)
#[allow(dead_code)]
const MIDAS_SMALL_URL: &str = "https://huggingface.co/depth-anything/Depth-Anything-V2-Small/resolve/main/depth_anything_v2_vits.onnx";

// Face detection - UltraFace is small (~1.2MB)
const ULTRAFACE_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/body_analysis/ultraface/models/version-RFB-320.onnx";

// Voice activity detection - Silero VAD (~2.3MB)
const SILERO_VAD_URL: &str =
    "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx";

// Image classification - SqueezeNet is small (~5MB)
const SQUEEZENET_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/classification/squeezenet/model/squeezenet1.0-12.onnx";

// Object detection - TinyYOLOv2 (~63MB but good for testing)
const TINY_YOLOV2_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/tiny-yolov2/model/tinyyolov2-8.onnx";

// Emotion recognition - small (~34MB)
const EMOTION_FERPLUS_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/body_analysis/emotion_ferplus/model/emotion-ferplus-8.onnx";

// Face embedding - ArcFace (~248MB - large)
#[allow(dead_code)]
const ARCFACE_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/body_analysis/arcface/model/arcfaceresnet100-8.onnx";

// MobileNet for faster classification testing (~13MB)
const MOBILENET_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx";

// Segmentation - FCN ResNet-50-int8 is quantized and smaller (~34MB)
const FCN_INT8_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/fcn/model/fcn-resnet50-12-int8.onnx";

#[derive(Clone, Copy)]
struct ModelZooCase {
    name: &'static str,
    url: &'static str,
    filename: &'static str,
}

const ONNX_MODEL_ZOO_OBJECT_DETECTION_MODELS: &[ModelZooCase] = &[
    ModelZooCase {
        name: "Faster R-CNN 10",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/faster-rcnn/model/FasterRCNN-10.onnx",
        filename: "FasterRCNN-10.onnx",
    },
    ModelZooCase {
        name: "Faster R-CNN 12 int8",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/faster-rcnn/model/FasterRCNN-12-int8.onnx",
        filename: "FasterRCNN-12-int8.onnx",
    },
    ModelZooCase {
        name: "Faster R-CNN 12 qdq",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/faster-rcnn/model/FasterRCNN-12-qdq.onnx",
        filename: "FasterRCNN-12-qdq.onnx",
    },
    ModelZooCase {
        name: "Faster R-CNN 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/faster-rcnn/model/FasterRCNN-12.onnx",
        filename: "FasterRCNN-12.onnx",
    },
    ModelZooCase {
        name: "Mask R-CNN 10",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/mask-rcnn/model/MaskRCNN-10.onnx",
        filename: "MaskRCNN-10.onnx",
    },
    ModelZooCase {
        name: "Mask R-CNN 12 int8",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/mask-rcnn/model/MaskRCNN-12-int8.onnx",
        filename: "MaskRCNN-12-int8.onnx",
    },
    ModelZooCase {
        name: "Mask R-CNN 12 qdq",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/mask-rcnn/model/MaskRCNN-12-qdq.onnx",
        filename: "MaskRCNN-12-qdq.onnx",
    },
    ModelZooCase {
        name: "Mask R-CNN 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/mask-rcnn/model/MaskRCNN-12.onnx",
        filename: "MaskRCNN-12.onnx",
    },
    ModelZooCase {
        name: "RetinaNet 9",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/retinanet/model/retinanet-9.onnx",
        filename: "retinanet-9.onnx",
    },
    ModelZooCase {
        name: "SSD-MobileNetV1 10",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1/model/ssd_mobilenet_v1_10.onnx",
        filename: "ssd_mobilenet_v1_10.onnx",
    },
    ModelZooCase {
        name: "SSD-MobileNetV1 12 int8",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1/model/ssd_mobilenet_v1_12-int8.onnx",
        filename: "ssd_mobilenet_v1_12-int8.onnx",
    },
    ModelZooCase {
        name: "SSD-MobileNetV1 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1/model/ssd_mobilenet_v1_12.onnx",
        filename: "ssd_mobilenet_v1_12.onnx",
    },
    ModelZooCase {
        name: "SSD-MobileNetV1 13 qdq",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1/model/ssd_mobilenet_v1_13-qdq.onnx",
        filename: "ssd_mobilenet_v1_13-qdq.onnx",
    },
    ModelZooCase {
        name: "SSD 10",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd/model/ssd-10.onnx",
        filename: "ssd-10.onnx",
    },
    ModelZooCase {
        name: "SSD 12 int8",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd/model/ssd-12-int8.onnx",
        filename: "ssd-12-int8.onnx",
    },
    ModelZooCase {
        name: "SSD 12 qdq",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd/model/ssd-12-qdq.onnx",
        filename: "ssd-12-qdq.onnx",
    },
    ModelZooCase {
        name: "SSD 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/ssd/model/ssd-12.onnx",
        filename: "ssd-12.onnx",
    },
    ModelZooCase {
        name: "Tiny YOLOv2 7",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/tiny-yolov2/model/tinyyolov2-7.onnx",
        filename: "tinyyolov2-7.onnx",
    },
    ModelZooCase {
        name: "Tiny YOLOv2 8",
        url: TINY_YOLOV2_URL,
        filename: "tinyyolov2-8.onnx",
    },
    ModelZooCase {
        name: "Tiny YOLOv3 11",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/tiny-yolov3/model/tiny-yolov3-11.onnx",
        filename: "tiny-yolov3-11.onnx",
    },
    ModelZooCase {
        name: "YOLOv2 COCO 9",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/yolov2-coco/model/yolov2-coco-9.onnx",
        filename: "yolov2-coco-9.onnx",
    },
    ModelZooCase {
        name: "YOLOv3 10",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/yolov3/model/yolov3-10.onnx",
        filename: "yolov3-10.onnx",
    },
    ModelZooCase {
        name: "YOLOv3 12 int8",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/yolov3/model/yolov3-12-int8.onnx",
        filename: "yolov3-12-int8.onnx",
    },
    ModelZooCase {
        name: "YOLOv3 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/yolov3/model/yolov3-12.onnx",
        filename: "yolov3-12.onnx",
    },
    ModelZooCase {
        name: "YOLOv4",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/yolov4/model/yolov4.onnx",
        filename: "yolov4.onnx",
    },
];

const ONNX_MODEL_ZOO_SEGMENTATION_MODELS: &[ModelZooCase] = &[
    ModelZooCase {
        name: "DUC ResNet101 12 int8",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/duc/model/ResNet101-DUC-12-int8.onnx",
        filename: "ResNet101-DUC-12-int8.onnx",
    },
    ModelZooCase {
        name: "DUC ResNet101 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/duc/model/ResNet101-DUC-12.onnx",
        filename: "ResNet101-DUC-12.onnx",
    },
    ModelZooCase {
        name: "DUC ResNet101 7",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/duc/model/ResNet101-DUC-7.onnx",
        filename: "ResNet101-DUC-7.onnx",
    },
    ModelZooCase {
        name: "FCN ResNet101 11",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/fcn/model/fcn-resnet101-11.onnx",
        filename: "fcn-resnet101-11.onnx",
    },
    ModelZooCase {
        name: "FCN ResNet50 11",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/fcn/model/fcn-resnet50-11.onnx",
        filename: "fcn-resnet50-11.onnx",
    },
    ModelZooCase {
        name: "FCN ResNet50 12 int8",
        url: FCN_INT8_URL,
        filename: "fcn-resnet50-12-int8.onnx",
    },
    ModelZooCase {
        name: "FCN ResNet50 12 qdq",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/fcn/model/fcn-resnet50-12-qdq.onnx",
        filename: "fcn-resnet50-12-qdq.onnx",
    },
    ModelZooCase {
        name: "FCN ResNet50 12",
        url: "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/fcn/model/fcn-resnet50-12.onnx",
        filename: "fcn-resnet50-12.onnx",
    },
];

// NER - BERT-base-NER ONNX (~430MB - requires HuggingFace)
// Note: This requires authentication for HuggingFace, so we use a smaller test model
#[allow(dead_code)]
const BERT_NER_URL: &str =
    "https://huggingface.co/dslim/bert-base-NER/resolve/main/onnx/model.onnx";

// ============================================================================
// ONNX Session Tests
// ============================================================================

#[cfg(feature = "execute")]
mod onnx_session_tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_load_ultraface_model() {
        let model_path = download_if_missing(ULTRAFACE_URL, "ultraface_RFB_320.onnx");

        let session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        println!("Model inputs:");
        for input in session.inputs() {
            println!("  - {} {:?}", input.name(), input.dtype());
        }

        println!("Model outputs:");
        for output in session.outputs() {
            println!("  - {} {:?}", output.name(), output.dtype());
        }

        assert!(!session.inputs().is_empty(), "Model should have inputs");
        assert!(!session.outputs().is_empty(), "Model should have outputs");
    }

    #[test]
    #[ignore]
    fn test_load_silero_vad_model() {
        let model_path = download_if_missing(SILERO_VAD_URL, "silero_vad.onnx");

        let session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        println!("Silero VAD inputs:");
        for input in session.inputs() {
            println!("  - {} {:?}", input.name(), input.dtype());
        }

        println!("Silero VAD outputs:");
        for output in session.outputs() {
            println!("  - {} {:?}", output.name(), output.dtype());
        }

        // Silero VAD should have specific inputs
        let input_names: Vec<&str> = session.inputs().iter().map(|input| input.name()).collect();
        println!("Input names: {:?}", input_names);

        assert!(!session.inputs().is_empty());
    }

    #[test]
    #[ignore]
    fn test_load_squeezenet_model() {
        let model_path = download_if_missing(SQUEEZENET_URL, "squeezenet1.0-12.onnx");

        let session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        println!("SqueezeNet inputs:");
        for input in session.inputs() {
            println!("  - {} {:?}", input.name(), input.dtype());
        }

        println!("SqueezeNet outputs:");
        for output in session.outputs() {
            println!("  - {} {:?}", output.name(), output.dtype());
        }

        assert!(!session.inputs().is_empty(), "Model should have inputs");
        assert!(!session.outputs().is_empty(), "Model should have outputs");
    }
}
// ============================================================================

#[cfg(feature = "execute")]
mod face_detection_tests {
    use super::*;
    use flow_like_model_provider::ml::{
        ndarray::Array4,
        ort::{inputs, value::Value},
    };
    use image::GenericImageView;

    #[test]
    #[ignore]
    fn test_ultraface_inference() {
        let model_path = download_if_missing(ULTRAFACE_URL, "ultraface_RFB_320.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // Create test image
        let img = create_face_test_image();
        let (orig_w, orig_h) = img.dimensions();
        println!("Original image size: {}x{}", orig_w, orig_h);

        // UltraFace expects 320x240 input
        let input_w = 320u32;
        let input_h = 240u32;

        let resized = img.resize_exact(input_w, input_h, image::imageops::FilterType::Triangle);
        let rgb = resized.to_rgb8();

        // Create input tensor [1, 3, H, W] with mean subtraction
        let mut input = Array4::<f32>::zeros((1, 3, input_h as usize, input_w as usize));
        for y in 0..input_h {
            for x in 0..input_w {
                let pixel = rgb.get_pixel(x, y);
                // UltraFace normalization: (pixel - 127) / 128
                input[[0, 0, y as usize, x as usize]] = (pixel[0] as f32 - 127.0) / 128.0;
                input[[0, 1, y as usize, x as usize]] = (pixel[1] as f32 - 127.0) / 128.0;
                input[[0, 2, y as usize, x as usize]] = (pixel[2] as f32 - 127.0) / 128.0;
            }
        }

        let input_value = Value::from_array(input).expect("Failed to create input tensor");

        // Run inference
        let outputs = session
            .run(inputs!["input" => input_value])
            .expect("Inference failed");

        println!("Output keys: {:?}", outputs.keys().collect::<Vec<_>>());

        // UltraFace outputs: scores and boxes
        for (name, tensor) in outputs.iter() {
            if let Ok(arr) = tensor.try_extract_array::<f32>() {
                println!("Output '{}' shape: {:?}", name, arr.shape());
            }
        }

        // Verify we got outputs (check by trying to iterate)
        let output_count = outputs.iter().count();
        assert!(output_count > 0, "Should have outputs");
    }
}

// ============================================================================
// Audio/VAD Tests
// ============================================================================

#[cfg(feature = "execute")]
mod vad_tests {
    use super::*;
    use flow_like_model_provider::ml::{
        ndarray::{Array1, Array3},
        ort::{inputs, value::Value},
    };

    #[test]
    #[ignore]
    fn test_silero_vad_inference() {
        let model_path = download_if_missing(SILERO_VAD_URL, "silero_vad.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // Print model info
        println!("Model inputs:");
        for input in session.inputs().iter() {
            println!("  - {} : {:?}", input.name(), input.dtype());
        }
        println!("Model outputs:");
        for output in session.outputs().iter() {
            println!("  - {} : {:?}", output.name(), output.dtype());
        }

        // Create fake audio chunk (512 samples at 16kHz = 32ms)
        let chunk_size = 512;
        let audio_chunk: Vec<f32> = (0..chunk_size)
            .map(|i| (i as f32 * 0.01).sin() * 0.5)
            .collect();

        // Silero VAD inputs (v5 model uses 3D state tensor)
        // input: [batch, samples] = [1, 512]
        let input = ndarray::Array2::from_shape_vec((1, chunk_size), audio_chunk)
            .expect("Failed to create input array");
        // state: [2, batch, 128] - NOT [batch, 2, 128]!
        let state = Array3::<f32>::zeros((2, 1, 128));
        // sr is a scalar tensor
        let sr = Array1::from_elem(1, 16000i64);

        let input_value = Value::from_array(input).expect("Failed to create input tensor");
        let state_value = Value::from_array(state).expect("Failed to create state tensor");
        let sr_value = Value::from_array(sr).expect("Failed to create sr tensor");

        // Run inference
        let outputs = session
            .run(inputs![
                "input" => input_value,
                "state" => state_value,
                "sr" => sr_value
            ])
            .expect("Inference failed");

        println!("VAD output keys: {:?}", outputs.keys().collect::<Vec<_>>());

        // Check output
        if let Some(output) = outputs.get("output")
            && let Ok(arr) = output.try_extract_array::<f32>()
        {
            println!("Speech probability shape: {:?}", arr.shape());
            println!("Speech probability: {:?}", arr);

            // Should be a probability between 0 and 1
            if let Some(&prob) = arr.first() {
                assert!(
                    (0.0..=1.0).contains(&prob),
                    "Probability should be in [0, 1], got {}",
                    prob
                );
            }
        }

        // Check state output
        if let Some(state_out) = outputs.get("stateN")
            && let Ok(arr) = state_out.try_extract_array::<f32>()
        {
            println!("New state shape: {:?}", arr.shape());
            // State output is [2, batch, 128]
            assert_eq!(
                arr.shape(),
                &[2, 1, 128],
                "State shape should be [2, 1, 128]"
            );
        }
    }
}

// ============================================================================
// Feature Extraction Tests
// ============================================================================

#[cfg(feature = "execute")]
mod feature_tests {
    #[allow(unused_imports)]
    use super::*;
    use flow_like_catalog_onnx::feature::FeatureVector;

    #[test]
    fn test_feature_vector_operations() {
        let v1 = FeatureVector::new(vec![1.0, 0.0, 0.0]);
        let v2 = FeatureVector::new(vec![0.0, 1.0, 0.0]);
        let v3 = FeatureVector::new(vec![1.0, 0.0, 0.0]);

        // Cosine similarity
        let sim_same = v1.cosine_similarity(&v3);
        assert!(
            (sim_same - 1.0).abs() < 0.001,
            "Same vectors should have similarity 1.0"
        );

        let sim_ortho = v1.cosine_similarity(&v2);
        assert!(
            sim_ortho.abs() < 0.001,
            "Orthogonal vectors should have similarity 0.0"
        );

        // L2 distance
        let dist_same = v1.l2_distance(&v3);
        assert!(
            dist_same.abs() < 0.001,
            "Same vectors should have distance 0.0"
        );

        let dist_ortho = v1.l2_distance(&v2);
        let expected_dist = 2.0f32.sqrt();
        assert!(
            (dist_ortho - expected_dist).abs() < 0.001,
            "Distance should be sqrt(2)"
        );

        // Normalization
        let mut v4 = FeatureVector::new(vec![3.0, 4.0]);
        v4.normalize();
        let norm: f32 = v4.values.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.001,
            "Normalized vector should have unit length"
        );
    }
}

// ============================================================================
// Type Serialization Tests
// ============================================================================

mod serialization_tests {
    use flow_like_catalog_onnx::{
        audio::{AudioData, SpeechSegment, VadResult},
        depth::DepthMap,
        face::{DetectedFace, FaceEmbedding, FaceLandmarks, LandmarkType},
        ocr::{RecognizedText, TextRegion},
    };

    #[test]
    fn test_audio_data_roundtrip() {
        let audio = AudioData::new(16000, 1, vec![0.1, 0.2, 0.3, 0.4]);
        let json = serde_json::to_string(&audio).unwrap();
        let decoded: AudioData = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.sample_rate, 16000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn test_vad_result_roundtrip() {
        let result = VadResult {
            segments: vec![
                SpeechSegment {
                    start: 0.5,
                    end: 1.5,
                    confidence: 0.95,
                },
                SpeechSegment {
                    start: 2.0,
                    end: 3.0,
                    confidence: 0.88,
                },
            ],
            probabilities: vec![0.1, 0.8, 0.9, 0.3],
            frame_duration: 0.032,
        };

        let json = serde_json::to_string(&result).unwrap();
        let decoded: VadResult = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.segments.len(), 2);
        assert_eq!(decoded.probabilities.len(), 4);
        assert!((decoded.frame_duration - 0.032).abs() < 0.001);
    }

    #[test]
    fn test_depth_map_roundtrip() {
        let depth = DepthMap {
            values: vec![0.1, 0.2, 0.3, 0.4],
            width: 2,
            height: 2,
            min_depth: 0.1,
            max_depth: 0.4,
        };

        let json = serde_json::to_string(&depth).unwrap();
        let decoded: DepthMap = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.get_depth(0, 0), Some(0.1));
        assert_eq!(decoded.get_depth(1, 1), Some(0.4));
    }

    #[test]
    fn test_detected_face_roundtrip() {
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
        let decoded: DetectedFace = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.bbox, [10.0, 20.0, 100.0, 100.0]);
        assert_eq!(decoded.confidence, 0.98);
        assert!(decoded.landmarks.is_some());
    }

    #[test]
    fn test_face_embedding_roundtrip() {
        let emb = FaceEmbedding {
            embedding: vec![0.1, 0.2, 0.3, 0.4, 0.5],
            dimension: 5,
        };

        let json = serde_json::to_string(&emb).unwrap();
        let decoded: FaceEmbedding = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.dimension, 5);
        assert_eq!(decoded.embedding.len(), 5);
    }

    #[test]
    fn test_ocr_types_roundtrip() {
        let region = TextRegion {
            bbox: [10.0, 20.0, 100.0, 30.0],
            polygon: [[10.0, 20.0], [110.0, 20.0], [110.0, 50.0], [10.0, 50.0]],
            confidence: 0.95,
        };

        let json = serde_json::to_string(&region).unwrap();
        let decoded: TextRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.confidence, 0.95);

        let text = RecognizedText {
            text: "Hello World".to_string(),
            confidence: 0.92,
            char_confidences: vec![0.95, 0.90],
        };

        let json = serde_json::to_string(&text).unwrap();
        let decoded: RecognizedText = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.text, "Hello World");
    }
}

// ============================================================================
// Audio Processing Tests
// ============================================================================

mod audio_processing_tests {
    use flow_like_catalog_onnx::audio::AudioData;

    #[test]
    fn test_audio_resample_downsample() {
        // Create 1 second of audio at 44100 Hz
        let samples: Vec<f32> = (0..44100)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 44100.0).sin())
            .collect();

        let audio = AudioData::new(44100, 1, samples);
        assert!((audio.duration_secs - 1.0).abs() < 0.01);

        // Resample to 16000 Hz
        let resampled = audio.resample(16000);
        assert_eq!(resampled.sample_rate, 16000);

        // Duration should be approximately the same
        assert!((resampled.duration_secs - 1.0).abs() < 0.1);

        // Should have fewer samples
        assert!(resampled.samples.len() < audio.samples.len());
    }

    #[test]
    fn test_audio_stereo_to_mono() {
        // Stereo: L=1.0, R=0.0, L=0.5, R=0.5
        let samples = vec![1.0, 0.0, 0.5, 0.5];
        let audio = AudioData::new(16000, 2, samples);

        let mono = audio.to_mono();
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.samples.len(), 2);
        assert!((mono.samples[0] - 0.5).abs() < 0.001); // avg(1.0, 0.0) = 0.5
        assert!((mono.samples[1] - 0.5).abs() < 0.001); // avg(0.5, 0.5) = 0.5
    }
}

// ============================================================================
// Image Classification Tests
// ============================================================================

#[cfg(feature = "execute")]
mod classification_tests {
    use super::*;
    use flow_like_model_provider::ml::{
        ndarray::Array4,
        ort::{inputs, value::Value},
    };
    use image::GenericImageView;

    #[test]
    #[ignore]
    fn test_squeezenet_inference() {
        let model_path = download_if_missing(SQUEEZENET_URL, "squeezenet1.0-12.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // Print model info
        println!("SqueezeNet inputs:");
        for input in session.inputs().iter() {
            println!("  - {} : {:?}", input.name(), input.dtype());
        }

        // Create test image (224x224 for SqueezeNet)
        let img = create_test_image(224, 224);
        let (w, h) = img.dimensions();
        let rgb = img.to_rgb8();

        // SqueezeNet expects [1, 3, 224, 224] NCHW format
        // Normalization: ImageNet mean/std
        let mean = [0.485, 0.456, 0.406];
        let std = [0.229, 0.224, 0.225];

        let mut input = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
        for y in 0..h {
            for x in 0..w {
                let pixel = rgb.get_pixel(x, y);
                // Normalize: (pixel / 255 - mean) / std
                input[[0, 0, y as usize, x as usize]] =
                    ((pixel[0] as f32 / 255.0) - mean[0]) / std[0];
                input[[0, 1, y as usize, x as usize]] =
                    ((pixel[1] as f32 / 255.0) - mean[1]) / std[1];
                input[[0, 2, y as usize, x as usize]] =
                    ((pixel[2] as f32 / 255.0) - mean[2]) / std[2];
            }
        }

        let input_value = Value::from_array(input).expect("Failed to create input tensor");

        // Run inference (SqueezeNet uses "data_0" as input name)
        let outputs = session
            .run(inputs!["data_0" => input_value])
            .expect("Inference failed");

        println!("Output keys: {:?}", outputs.keys().collect::<Vec<_>>());

        // SqueezeNet outputs 1000 class logits
        for (name, tensor) in outputs.iter() {
            if let Ok(arr) = tensor.try_extract_array::<f32>() {
                println!("Output '{}' shape: {:?}", name, arr.shape());

                // Find top prediction
                let flat = arr.as_slice().unwrap();
                let (max_idx, max_val) = flat
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                    .unwrap();
                println!(
                    "Top prediction: class {} with logit {:.4}",
                    max_idx, max_val
                );

                // Shape should be [1, 1000] or [1, 1000, 1, 1]
                assert!(arr.len() == 1000, "Should have 1000 class outputs");
            }
        }
    }

    #[test]
    #[ignore]
    fn test_mobilenet_inference() {
        let model_path = download_if_missing(MOBILENET_URL, "mobilenetv2-12.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // Print model info
        println!("MobileNetV2 inputs:");
        for input in session.inputs().iter() {
            println!("  - {} : {:?}", input.name(), input.dtype());
        }
        println!("MobileNetV2 outputs:");
        for output in session.outputs().iter() {
            println!("  - {} : {:?}", output.name(), output.dtype());
        }

        // Create test image (224x224 for MobileNet)
        let img = create_test_image(224, 224);
        let (w, h) = img.dimensions();
        let rgb = img.to_rgb8();

        // MobileNet expects [1, 3, 224, 224] NCHW format
        let mean = [0.485, 0.456, 0.406];
        let std = [0.229, 0.224, 0.225];

        let mut input = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
        for y in 0..h {
            for x in 0..w {
                let pixel = rgb.get_pixel(x, y);
                input[[0, 0, y as usize, x as usize]] =
                    ((pixel[0] as f32 / 255.0) - mean[0]) / std[0];
                input[[0, 1, y as usize, x as usize]] =
                    ((pixel[1] as f32 / 255.0) - mean[1]) / std[1];
                input[[0, 2, y as usize, x as usize]] =
                    ((pixel[2] as f32 / 255.0) - mean[2]) / std[2];
            }
        }

        let input_value = Value::from_array(input).expect("Failed to create input tensor");

        // Get input name from session (clone to avoid borrow conflict)
        let input_name = session.inputs()[0].name().to_owned();
        let outputs = session
            .run(inputs![input_name.as_str() => input_value])
            .expect("Inference failed");

        for (name, tensor) in outputs.iter() {
            if let Ok(arr) = tensor.try_extract_array::<f32>() {
                println!("Output '{}' shape: {:?}", name, arr.shape());
                assert!(arr.len() == 1000, "Should have 1000 class outputs");
            }
        }
    }
}

// ============================================================================
// Object Detection Tests
// ============================================================================

#[cfg(feature = "execute")]
mod object_detection_tests {
    use super::*;
    use flow_like_catalog_onnx::{Provider, detection::ObjectDetection, load::determine_provider};
    use flow_like_model_provider::ml::{
        ndarray::Array4,
        ort::{inputs, value::Value},
    };
    use image::GenericImageView;

    #[test]
    #[ignore]
    fn test_tiny_yolov2_load() {
        let model_path = download_if_missing(TINY_YOLOV2_URL, "tinyyolov2-8.onnx");

        let session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        println!("TinyYOLOv2 inputs:");
        for input in session.inputs() {
            println!("  - {} {:?}", input.name(), input.dtype());
        }
        println!("TinyYOLOv2 outputs:");
        for output in session.outputs() {
            println!("  - {} {:?}", output.name(), output.dtype());
        }

        assert!(!session.inputs().is_empty());
        assert!(!session.outputs().is_empty());

        let provider = determine_provider(&session).expect("provider detection should succeed");
        assert!(
            matches!(provider, Provider::YoloV2GridLike(_)),
            "TinyYOLOv2 should be recognized as YOLOv2 grid provider"
        );
    }

    #[test]
    #[ignore]
    fn test_tiny_yolov2_inference() {
        let model_path = download_if_missing(TINY_YOLOV2_URL, "tinyyolov2-8.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // TinyYOLOv2 expects 416x416 input
        let img = create_test_image(416, 416);
        let (w, h) = img.dimensions();
        let rgb = img.to_rgb8();

        // TinyYOLOv2 expects [1, 3, 416, 416] NCHW format, normalized to [0,1]
        let mut input = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
        for y in 0..h {
            for x in 0..w {
                let pixel = rgb.get_pixel(x, y);
                input[[0, 0, y as usize, x as usize]] = pixel[0] as f32 / 255.0;
                input[[0, 1, y as usize, x as usize]] = pixel[1] as f32 / 255.0;
                input[[0, 2, y as usize, x as usize]] = pixel[2] as f32 / 255.0;
            }
        }

        let input_value = Value::from_array(input).expect("Failed to create input tensor");

        let input_name = session.inputs()[0].name().to_owned();
        {
            let outputs = session
                .run(inputs![input_name.as_str() => input_value])
                .expect("Inference failed");

            println!(
                "TinyYOLOv2 output keys: {:?}",
                outputs.keys().collect::<Vec<_>>()
            );

            for (name, tensor) in outputs.iter() {
                if let Ok(arr) = tensor.try_extract_array::<f32>() {
                    println!("Output '{}' shape: {:?}", name, arr.shape());
                    // TinyYOLOv2 outputs detection grid
                    assert!(arr.shape().len() >= 3, "Should have detection grid output");
                }
            }
        }

        let provider = determine_provider(&session).expect("provider detection should succeed");
        let bboxes = match provider {
            Provider::YoloV2GridLike(provider) => provider
                .run(&mut session, &img, 1.0, 0.7, 5)
                .expect("Object-detection provider should run TinyYOLOv2"),
            _ => panic!("TinyYOLOv2 should be recognized as a YOLOv2 grid provider"),
        };
        assert!(bboxes.len() <= 5);
    }

    #[test]
    #[ignore]
    fn test_yolov3_boxes_stay_in_wide_image_bounds() {
        let model_path = download_if_missing(
            "https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/yolov3/model/yolov3-12.onnx",
            "yolov3-12.onnx",
        );

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");
        let img = create_test_image(640, 360);
        let provider = determine_provider(&session).expect("provider detection should succeed");
        let bboxes = match provider {
            Provider::YoloV3Like(provider) => provider
                .run(&mut session, &img, 0.0, 0.7, 20)
                .expect("Object-detection provider should run YOLOv3"),
            _ => panic!("YOLOv3 should be recognized as a YOLOv3 provider"),
        };

        if bboxes.is_empty() {
            println!("YOLOv3 returned no selected boxes for the synthetic wide test image");
            return;
        }

        for bbox in bboxes {
            assert!(
                bbox.x1 >= 0.0
                    && bbox.x1 <= img.width() as f32
                    && bbox.x2 >= 0.0
                    && bbox.x2 <= img.width() as f32
                    && bbox.y1 >= 0.0
                    && bbox.y1 <= img.height() as f32
                    && bbox.y2 >= 0.0
                    && bbox.y2 <= img.height() as f32
                    && bbox.x2 >= bbox.x1
                    && bbox.y2 >= bbox.y1,
                "YOLOv3 returned out-of-bounds or inverted box: {:?}",
                bbox
            );
        }
    }

    #[test]
    fn test_onnx_model_zoo_manifest_counts() {
        assert_eq!(ONNX_MODEL_ZOO_OBJECT_DETECTION_MODELS.len(), 25);
        assert_eq!(ONNX_MODEL_ZOO_SEGMENTATION_MODELS.len(), 8);
    }

    #[test]
    #[ignore]
    fn test_onnx_model_zoo_object_detection_weights_on_object_detection_node() {
        let img = create_test_image(416, 416);

        for case in ONNX_MODEL_ZOO_OBJECT_DETECTION_MODELS {
            println!("--- {} ---", case.name);
            let model_path = download_if_missing(case.url, case.filename);
            let mut session = configured_session_builder()
                .expect("Failed to create session builder")
                .commit_from_file(&model_path)
                .unwrap_or_else(|err| panic!("Failed to load {}: {err}", case.name));

            let provider = determine_provider(&session).unwrap_or_else(|err| {
                panic!("Failed to determine provider for {}: {err}", case.name)
            });

            let bboxes = match provider {
                Provider::DfineLike(provider) => provider.run(&mut session, &img, 1.0, 0.7, 5),
                Provider::YoloLike(provider) => provider.run(&mut session, &img, 1.0, 0.7, 5),
                Provider::BoxLabelsScoresLike(provider) => {
                    provider.run(&mut session, &img, 1.0, 0.7, 5)
                }
                Provider::SsdMobileNetLike(provider) => {
                    provider.run(&mut session, &img, 1.0, 0.7, 5)
                }
                Provider::YoloV2GridLike(provider) => provider.run(&mut session, &img, 1.0, 0.7, 5),
                Provider::YoloV3Like(provider) => provider.run(&mut session, &img, 1.0, 0.7, 5),
                Provider::YoloV4Like(provider) => provider.run(&mut session, &img, 1.0, 0.7, 5),
                Provider::RetinaNetLike(provider) => provider.run(&mut session, &img, 1.0, 0.7, 5),
                Provider::TimmLike(_) | Provider::Generic => {
                    panic!(
                        "{} was not recognized as an object-detection provider",
                        case.name
                    )
                }
            }
            .unwrap_or_else(|err| panic!("Object detection failed for {}: {err}", case.name));

            assert!(
                bboxes.len() <= 5,
                "{} returned more boxes than requested",
                case.name
            );
            println!("{} returned {} boxes", case.name, bboxes.len());
        }
    }

    #[test]
    #[ignore]
    fn test_onnx_model_zoo_segmentation_weights_are_not_object_detection() {
        for case in ONNX_MODEL_ZOO_SEGMENTATION_MODELS {
            println!("--- {} ---", case.name);
            let model_path = download_if_missing(case.url, case.filename);
            let session = configured_session_builder()
                .expect("Failed to create session builder")
                .commit_from_file(&model_path)
                .unwrap_or_else(|err| panic!("Failed to load {}: {err}", case.name));

            let provider = determine_provider(&session).unwrap_or_else(|err| {
                panic!("Failed to determine provider for {}: {err}", case.name)
            });
            assert!(
                matches!(provider, Provider::Generic),
                "{} is a segmentation model and should not be routed to object detection",
                case.name
            );
        }
    }
}

// ============================================================================
// Emotion Recognition Tests
// ============================================================================

#[cfg(feature = "execute")]
mod emotion_tests {
    use super::*;
    use flow_like_model_provider::ml::{
        ndarray::Array4,
        ort::{inputs, value::Value},
    };

    #[test]
    #[ignore]
    fn test_emotion_ferplus_load() {
        let model_path = download_if_missing(EMOTION_FERPLUS_URL, "emotion-ferplus-8.onnx");

        let session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        println!("Emotion FERPlus inputs:");
        for input in session.inputs() {
            println!("  - {} {:?}", input.name(), input.dtype());
        }
        println!("Emotion FERPlus outputs:");
        for output in session.outputs() {
            println!("  - {} {:?}", output.name(), output.dtype());
        }

        assert!(!session.inputs().is_empty());
        assert!(!session.outputs().is_empty());
    }

    #[test]
    #[ignore]
    fn test_emotion_ferplus_inference() {
        let model_path = download_if_missing(EMOTION_FERPLUS_URL, "emotion-ferplus-8.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // Emotion FERPlus expects 64x64 grayscale input
        // Input shape: [1, 1, 64, 64]
        let img = create_face_test_image();
        let gray = img.to_luma8();
        let resized = image::imageops::resize(&gray, 64, 64, image::imageops::FilterType::Triangle);

        let mut input = Array4::<f32>::zeros((1, 1, 64, 64));
        for y in 0..64 {
            for x in 0..64 {
                let pixel = resized.get_pixel(x, y);
                input[[0, 0, y as usize, x as usize]] = pixel[0] as f32;
            }
        }

        let input_value = Value::from_array(input).expect("Failed to create input tensor");

        let input_name = session.inputs()[0].name().to_owned();
        let outputs = session
            .run(inputs![input_name.as_str() => input_value])
            .expect("Inference failed");

        println!(
            "Emotion output keys: {:?}",
            outputs.keys().collect::<Vec<_>>()
        );

        // Emotion labels: neutral, happiness, surprise, sadness, anger, disgust, fear, contempt
        let emotions = [
            "neutral",
            "happiness",
            "surprise",
            "sadness",
            "anger",
            "disgust",
            "fear",
            "contempt",
        ];

        for (name, tensor) in outputs.iter() {
            if let Ok(arr) = tensor.try_extract_array::<f32>() {
                println!("Output '{}' shape: {:?}", name, arr.shape());

                let flat = arr.as_slice().unwrap();
                if flat.len() == 8 {
                    let (max_idx, max_val) = flat
                        .iter()
                        .enumerate()
                        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                        .unwrap();
                    println!("Detected emotion: {} ({:.2}%)", emotions[max_idx], max_val);
                }

                assert_eq!(arr.len(), 8, "Should have 8 emotion classes");
            }
        }
    }
}

// ============================================================================
// Segmentation Tests
// ============================================================================

#[cfg(feature = "execute")]
mod segmentation_tests {
    use super::*;
    use flow_like_model_provider::ml::{
        ndarray::Array4,
        ort::{inputs, value::Value},
    };

    #[test]
    #[ignore]
    fn test_fcn_int8_load() {
        let model_path = download_if_missing(FCN_INT8_URL, "fcn-resnet50-12-int8.onnx");

        let session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        println!("FCN ResNet-50-int8 inputs:");
        for input in session.inputs() {
            println!("  - {} {:?}", input.name(), input.dtype());
        }
        println!("FCN ResNet-50-int8 outputs:");
        for output in session.outputs() {
            println!("  - {} {:?}", output.name(), output.dtype());
        }

        assert!(!session.inputs().is_empty());
        assert!(!session.outputs().is_empty());
    }

    #[test]
    #[ignore]
    fn test_fcn_int8_inference() {
        let model_path = download_if_missing(FCN_INT8_URL, "fcn-resnet50-12-int8.onnx");

        let mut session = configured_session_builder()
            .expect("Failed to create session builder")
            .commit_from_file(&model_path)
            .expect("Failed to load model");

        // FCN expects variable input size with min edge 520, but we use 224 for faster test
        let input_size = 224u32;
        let img = create_test_image(input_size, input_size);
        let rgb = img.to_rgb8();

        // FCN normalization: ImageNet mean/std
        let mean = [0.485, 0.456, 0.406];
        let std = [0.229, 0.224, 0.225];

        let mut input = Array4::<f32>::zeros((1, 3, input_size as usize, input_size as usize));
        for y in 0..input_size {
            for x in 0..input_size {
                let pixel = rgb.get_pixel(x, y);
                input[[0, 0, y as usize, x as usize]] =
                    ((pixel[0] as f32 / 255.0) - mean[0]) / std[0];
                input[[0, 1, y as usize, x as usize]] =
                    ((pixel[1] as f32 / 255.0) - mean[1]) / std[1];
                input[[0, 2, y as usize, x as usize]] =
                    ((pixel[2] as f32 / 255.0) - mean[2]) / std[2];
            }
        }

        let input_value = Value::from_array(input).expect("Failed to create input tensor");

        let input_name = session.inputs()[0].name().to_owned();
        let outputs = session
            .run(inputs![input_name.as_str() => input_value])
            .expect("Inference failed");

        println!("FCN output keys: {:?}", outputs.keys().collect::<Vec<_>>());

        // FCN outputs: "out" with shape [N, 21, H, W]
        for (name, tensor) in outputs.iter() {
            if let Ok(arr) = tensor.try_extract_array::<f32>() {
                println!("Output '{}' shape: {:?}", name, arr.shape());
                // Should have 21 classes (VOC dataset)
                if name == "out" {
                    assert_eq!(arr.shape()[1], 21, "Should have 21 segmentation classes");
                }
            }
        }
    }
}

// ============================================================================
// NER Tests
// ============================================================================

#[cfg(feature = "execute")]
mod ner_tests {
    use flow_like_catalog_onnx::ner::{EntityLabel, merge_entities};

    #[test]
    fn test_entity_label_parsing() {
        assert_eq!(EntityLabel::from_str("O"), EntityLabel::O);
        assert!(matches!(EntityLabel::from_str("B-PER"), EntityLabel::Begin(t) if t == "PER"));
        assert!(matches!(EntityLabel::from_str("I-ORG"), EntityLabel::Inside(t) if t == "ORG"));
        assert!(matches!(EntityLabel::from_str("B-LOC"), EntityLabel::Begin(t) if t == "LOC"));
        assert!(matches!(EntityLabel::from_str("B-MISC"), EntityLabel::Begin(t) if t == "MISC"));
    }

    #[test]
    fn test_entity_merging() {
        let tokens = vec![
            "John".to_string(),
            "Smith".to_string(),
            "works".to_string(),
            "at".to_string(),
            "Google".to_string(),
            "in".to_string(),
            "New".to_string(),
            "York".to_string(),
        ];
        let labels = vec![
            EntityLabel::Begin("PER".to_string()),
            EntityLabel::Inside("PER".to_string()),
            EntityLabel::O,
            EntityLabel::O,
            EntityLabel::Begin("ORG".to_string()),
            EntityLabel::O,
            EntityLabel::Begin("LOC".to_string()),
            EntityLabel::Inside("LOC".to_string()),
        ];
        let confidences = vec![0.95, 0.92, 0.1, 0.1, 0.88, 0.1, 0.85, 0.83];
        let offsets = vec![
            (0, 4),
            (5, 10),
            (11, 16),
            (17, 19),
            (20, 26),
            (27, 29),
            (30, 33),
            (34, 38),
        ];
        let text = "John Smith works at Google in New York";

        let entities = merge_entities(&tokens, &labels, &confidences, Some(&offsets), text);

        assert_eq!(entities.len(), 3, "Should find 3 entities");

        // Person: John Smith
        assert_eq!(entities[0].text, "John Smith");
        assert_eq!(entities[0].entity_type, "PER");
        assert_eq!(entities[0].start_token, 0);
        assert_eq!(entities[0].end_token, 2);

        // Organization: Google
        assert_eq!(entities[1].text, "Google");
        assert_eq!(entities[1].entity_type, "ORG");

        // Location: New York
        assert_eq!(entities[2].text, "New York");
        assert_eq!(entities[2].entity_type, "LOC");
    }

    #[test]
    fn test_entity_label_type() {
        assert_eq!(
            EntityLabel::Begin("PER".to_string()).entity_type(),
            Some("PER")
        );
        assert_eq!(
            EntityLabel::Inside("PER".to_string()).entity_type(),
            Some("PER")
        );
        assert_eq!(
            EntityLabel::Begin("ORG".to_string()).entity_type(),
            Some("ORG")
        );
        assert_eq!(
            EntityLabel::Begin("LOC".to_string()).entity_type(),
            Some("LOC")
        );
        assert_eq!(EntityLabel::O.entity_type(), None);
    }

    #[test]
    fn test_is_beginning() {
        assert!(EntityLabel::Begin("PER".to_string()).is_beginning());
        assert!(EntityLabel::Begin("ORG".to_string()).is_beginning());
        assert!(EntityLabel::Begin("LOC".to_string()).is_beginning());
        assert!(EntityLabel::Begin("MISC".to_string()).is_beginning());
        assert!(!EntityLabel::Inside("PER".to_string()).is_beginning());
        assert!(!EntityLabel::O.is_beginning());
    }

    #[test]
    fn test_bioes_tagging() {
        // Test BIOES scheme: Single-token entity
        assert!(matches!(EntityLabel::from_str("S-PER"), EntityLabel::Single(t) if t == "PER"));
        assert!(matches!(EntityLabel::from_str("E-ORG"), EntityLabel::End(t) if t == "ORG"));

        // Single-token entity test
        let tokens = vec!["Paris".to_string()];
        let labels = vec![EntityLabel::Single("LOC".to_string())];
        let confidences = vec![0.99];
        let offsets = vec![(0, 5)];
        let text = "Paris";

        let entities = merge_entities(&tokens, &labels, &confidences, Some(&offsets), text);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].text, "Paris");
        assert_eq!(entities[0].entity_type, "LOC");
    }

    #[test]
    fn test_bilou_tagging() {
        // Test BILOU scheme
        assert!(matches!(EntityLabel::from_str("L-PER"), EntityLabel::Last(t) if t == "PER"));
        assert!(matches!(EntityLabel::from_str("U-ORG"), EntityLabel::Unit(t) if t == "ORG"));

        // Unit entity is like Single
        assert!(EntityLabel::Unit("LOC".to_string()).is_single());
        assert!(EntityLabel::Unit("LOC".to_string()).is_beginning());
    }
}

// ============================================================================
// Model Info Summary - Documents all tested models
// ============================================================================

/// This test runs all model loads and prints a comprehensive summary
/// Use: cargo test -p flow-like-catalog-onnx --test integration_tests --features execute -- --ignored test_all_models_summary --nocapture
#[cfg(feature = "execute")]
#[test]
#[ignore]
fn test_all_models_summary() {
    println!("\n================================================================================");
    println!("ONNX MODEL COMPATIBILITY SUMMARY");
    println!("================================================================================\n");

    let models = [
        (
            "UltraFace (Face Detection)",
            ULTRAFACE_URL,
            "ultraface_RFB_320.onnx",
        ),
        (
            "Silero VAD (Voice Activity)",
            SILERO_VAD_URL,
            "silero_vad.onnx",
        ),
        (
            "SqueezeNet (Classification)",
            SQUEEZENET_URL,
            "squeezenet1.0-12.onnx",
        ),
        (
            "MobileNetV2 (Classification)",
            MOBILENET_URL,
            "mobilenetv2-12.onnx",
        ),
        (
            "TinyYOLOv2 (Object Detection)",
            TINY_YOLOV2_URL,
            "tinyyolov2-8.onnx",
        ),
        (
            "Emotion FERPlus",
            EMOTION_FERPLUS_URL,
            "emotion-ferplus-8.onnx",
        ),
        (
            "FCN ResNet-50-int8 (Segmentation)",
            FCN_INT8_URL,
            "fcn-resnet50-12-int8.onnx",
        ),
    ];

    for (name, url, filename) in models {
        println!("--- {} ---", name);
        println!("URL: {}", url);

        match std::panic::catch_unwind(|| {
            let model_path = download_if_missing(url, filename);
            configured_session_builder()
                .expect("session builder")
                .commit_from_file(&model_path)
                .expect("load model")
        }) {
            Ok(session) => {
                println!("✓ Load: SUCCESS");
                println!("  Inputs:");
                for input in session.inputs() {
                    println!("    - {} {:?}", input.name(), input.dtype());
                }
                println!("  Outputs:");
                for output in session.outputs() {
                    println!("    - {} {:?}", output.name(), output.dtype());
                }
            }
            Err(e) => {
                println!("✗ Load: FAILED - {:?}", e);
            }
        }
        println!();
    }

    println!("================================================================================");
    println!("END SUMMARY");
    println!("================================================================================\n");
}
