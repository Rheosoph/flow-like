use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use flow_like_types::data_classification::DataClassification;
use serde_json::{Value, json};

use super::*;
use crate::files::handle_for;
use crate::outcome::{Status, ToolOutcome};
use crate::run;
use crate::testing::{FakeEnv, WORKSPACE};

#[derive(Default)]
struct FakeReader {
    answers: Mutex<VecDeque<BarcodeScan>>,
    requests: Mutex<Vec<BarcodeRequest>>,
}

impl FakeReader {
    fn answering(answers: impl IntoIterator<Item = BarcodeScan>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            requests: Mutex::default(),
        })
    }

    fn requests(&self) -> Vec<BarcodeRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

impl BarcodeReader for FakeReader {
    fn read(&self, _image: Bytes, request: &BarcodeRequest) -> Result<BarcodeScan> {
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
        Ok(self
            .answers
            .lock()
            .expect("answers")
            .pop_front()
            .unwrap_or_default())
    }
}

fn scan_of(codes: &[(&str, &str)]) -> BarcodeScan {
    BarcodeScan {
        width: 800,
        height: 600,
        codes: codes
            .iter()
            .map(|(format, text)| FoundCode {
                format: format.to_string(),
                text: text.to_string(),
                bounds: Some(CodeBox {
                    x: 10,
                    y: 20,
                    width: 100,
                    height: 100,
                }),
            })
            .collect(),
        ..BarcodeScan::default()
    }
}

async fn env_with(reader: Arc<FakeReader>) -> FakeEnv {
    let env = FakeEnv::new().await;
    *env.barcodes.lock().expect("barcodes") = Some(reader);
    env.put(WORKSPACE, "label.png", b"png", "image/png");
    env
}

async fn read(env: &FakeEnv, args: Value) -> ToolOutcome {
    run(BARCODE_READ, args, env).await
}

fn error_code(outcome: &ToolOutcome) -> &'static str {
    assert_eq!(outcome.status, Status::Error, "{:?}", outcome.result);
    outcome.error.as_ref().expect("error").code
}

#[tokio::test]
async fn reads_an_attachment_with_format_text_and_box() {
    let reader = FakeReader::answering([scan_of(&[("qr_code", "https://example.com/t/1")])]);
    let env = env_with(reader.clone()).await;
    env.put("ws-sender", "photo.jpg", b"jpeg", "image/jpeg");
    env.attach(
        7,
        "a1",
        "photo.jpg",
        handle_for("ws-sender", "photo.jpg", b"jpeg", "image/jpeg"),
    );

    let outcome = read(&env, json!({ "file": "#7.a1" })).await;
    assert_eq!(outcome.status, Status::Ok, "{:?}", outcome.error);
    assert_eq!(
        outcome.result["codes"],
        json!([{
            "format": "qr_code",
            "text": "https://example.com/t/1",
            "box": { "x": 10, "y": 20, "width": 100, "height": 100 },
        }])
    );
    assert_eq!(
        outcome.result["image"],
        json!({ "width": 800, "height": 600 })
    );
    assert_eq!(outcome.result["thorough"], false);
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    assert_eq!(outcome.result["untrusted"], true);
    assert_eq!(
        reader.requests(),
        [BarcodeRequest {
            formats: Vec::new(),
            thorough: false,
            max_codes: MAX_CODES,
        }]
    );
}

#[tokio::test]
async fn a_quick_search_that_finds_nothing_is_retried_thoroughly() {
    let reader = FakeReader::answering([scan_of(&[]), scan_of(&[("ean_13", "4006381333931")])]);
    let env = env_with(reader.clone()).await;

    let outcome = read(&env, json!({ "file": "label.png" })).await;
    assert_eq!(outcome.status, Status::Ok);
    assert_eq!(outcome.result["codes"][0]["text"], "4006381333931");
    assert_eq!(outcome.result["thorough"], true);
    let requests = reader.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].thorough && requests[1].thorough);
}

#[tokio::test]
async fn formats_and_thorough_go_to_the_reader_in_one_pass() {
    let reader = FakeReader::answering([scan_of(&[])]);
    let env = env_with(reader.clone()).await;

    let outcome = read(
        &env,
        json!({ "file": "label.png", "formats": ["ean_13", "qr_code"], "thorough": true }),
    )
    .await;
    assert_eq!(outcome.status, Status::Ok);
    assert_eq!(
        reader.requests(),
        [BarcodeRequest {
            formats: vec![BarcodeKind::Ean13, BarcodeKind::QrCode],
            thorough: true,
            max_codes: MAX_CODES,
        }]
    );

    let unknown = read(&env, json!({ "file": "label.png", "formats": ["barcode"] })).await;
    assert_eq!(error_code(&unknown), code::INVALID_ARGS);
}

#[tokio::test]
async fn nothing_found_says_what_helps_and_why_a_code_would_not_decode() {
    let env = env_with(FakeReader::answering([scan_of(&[]), scan_of(&[])])).await;
    let outcome = read(&env, json!({ "file": "label.png" })).await;
    assert_eq!(outcome.result["codes"], json!([]));
    assert!(outcome.warnings[0].starts_with("No code found"));

    let unreadable = BarcodeScan {
        unreadable: Some("checksum failed".into()),
        ..scan_of(&[])
    };
    let env = env_with(FakeReader::answering([scan_of(&[]), unreadable])).await;
    let outcome = read(&env, json!({ "file": "label.png" })).await;
    assert!(outcome.warnings[0].contains("checksum failed"));
}

#[tokio::test]
async fn scaled_reads_and_full_searches_warn() {
    let crowded: Vec<(&str, &str)> = vec![("qr_code", "x"); MAX_CODES];
    let scaled = BarcodeScan {
        scaled_to: Some((400, 300)),
        ..scan_of(&crowded)
    };
    let env = env_with(FakeReader::answering([scaled])).await;
    let outcome = read(&env, json!({ "file": "label.png" })).await;
    assert_eq!(outcome.warnings.len(), 3, "{:?}", outcome.warnings);
    assert!(outcome.warnings[0].contains("400×300 instead of 800×600"));
    assert!(outcome.warnings[1].contains(&MAX_CODES.to_string()));
}

#[tokio::test]
async fn the_image_label_carries_into_the_result() {
    let env = env_with(FakeReader::answering([scan_of(&[("qr_code", "x")])])).await;
    env.classify(WORKSPACE, "label.png", DataClassification::Confidential);
    let outcome = read(&env, json!({ "file": "label.png" })).await;
    assert_eq!(
        outcome.classification,
        Some(DataClassification::Confidential)
    );
}

#[tokio::test]
async fn pdfs_point_to_pdf_render_and_a_host_without_a_reader_says_so() {
    let reader = FakeReader::answering([]);
    let env = env_with(reader.clone()).await;
    env.put(WORKSPACE, "invoice.pdf", b"%PDF-1.7", "application/pdf");
    let outcome = read(&env, json!({ "file": "invoice.pdf" })).await;
    assert_eq!(error_code(&outcome), code::UNSUPPORTED_FILE);
    assert!(
        outcome
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("pdf_render"))
    );
    assert!(reader.requests().is_empty());

    let env = FakeEnv::new().await;
    env.put(WORKSPACE, "label.png", b"png", "image/png");
    let outcome = read(&env, json!({ "file": "label.png" })).await;
    assert_eq!(error_code(&outcome), code::NOT_YET);
}

#[test]
fn the_tool_needs_the_barcodes_capability_and_never_asks() {
    let spec = crate::spec(BARCODE_READ).expect("registered");
    assert_eq!(spec.capability, Some("barcodes"));
    assert_eq!(spec.approval, Approval::None);
    assert!(!spec.mutating);
    let formats = spec.input_schema.to_string();
    for kind in BarcodeKind::ALL {
        assert!(
            formats.contains(&format!("\"{}\"", kind.as_str())),
            "{kind:?}"
        );
    }
}

#[test]
fn kind_ids_are_the_wire_format() {
    for kind in BarcodeKind::ALL {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.as_str()));
        assert_eq!(
            serde_json::from_value::<BarcodeKind>(json!(kind.as_str())).unwrap(),
            kind
        );
    }
}

#[cfg(feature = "barcodes")]
mod decoder {
    use flow_like_catalog_media_image::image::content::read_barcodes::read_barcodes_in_luma;
    use flow_like_types::image::{
        DynamicImage, GenericImageView, GrayImage, ImageFormat, Luma, imageops,
    };
    use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};

    use super::*;

    fn code_image(text: &str, format: BarcodeFormat, scale: u32, height: Option<u32>) -> GrayImage {
        let hints = EncodeHints {
            Margin: Some("8".to_string()),
            ..EncodeHints::default()
        };
        let matrix = MultiFormatWriter
            .encode_with_hints(text, &format, 0, 0, &hints)
            .expect("encode");
        let image: DynamicImage = matrix.into();
        let (width, own_height) = image.dimensions();
        image
            .resize_exact(
                width * scale,
                height.unwrap_or(own_height * scale),
                imageops::FilterType::Nearest,
            )
            .to_luma8()
    }

    fn png(image: GrayImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("png");
        bytes.into_inner()
    }

    async fn real_env() -> FakeEnv {
        let env = FakeEnv::new().await;
        *env.barcodes.lock().expect("barcodes") = Some(Arc::new(CatalogBarcodes));
        env
    }

    #[tokio::test]
    async fn reads_a_qr_code_and_a_code_128_from_a_png() {
        let mut canvas = GrayImage::from_pixel(900, 320, Luma([255]));
        let qr = code_image("https://flow-like.com/b", BarcodeFormat::QR_CODE, 6, None);
        let line = code_image("SHIP-0042", BarcodeFormat::CODE_128, 3, Some(96));
        imageops::overlay(&mut canvas, &qr, 24, 24);
        imageops::overlay(&mut canvas, &line, 340, 96);
        let env = real_env().await;
        env.put(WORKSPACE, "parcel.png", &png(canvas), "image/png");

        let outcome = read(&env, json!({ "file": "parcel.png" })).await;
        assert_eq!(outcome.status, Status::Ok, "{:?}", outcome.error);
        let codes = outcome.result["codes"].as_array().expect("codes");
        let find = |text: &str| {
            codes
                .iter()
                .find(|code| code["text"] == text)
                .unwrap_or_else(|| panic!("{text} in {codes:?}"))
        };
        let qr = find("https://flow-like.com/b");
        assert_eq!(qr["format"], "qr_code");
        assert!(qr["box"]["x"].as_u64().unwrap() < 340);
        let line = find("SHIP-0042");
        assert_eq!(line["format"], "code_128");
        assert!(line["box"]["x"].as_u64().unwrap() >= 300);
        assert_eq!(
            outcome.result["image"],
            json!({ "width": 900, "height": 320 })
        );
    }

    #[tokio::test]
    async fn a_file_that_is_no_image_is_unsupported() {
        let env = real_env().await;
        env.put(WORKSPACE, "notes.txt", b"just words", "text/plain");
        let outcome = read(&env, json!({ "file": "notes.txt" })).await;
        assert_eq!(error_code(&outcome), code::UNSUPPORTED_FILE);
    }

    #[test]
    fn every_kind_is_a_format_the_decoder_accepts() {
        for kind in BarcodeKind::ALL {
            let request = BarcodeRequest {
                formats: vec![kind],
                thorough: false,
                max_codes: 1,
            };
            let blank = vec![255u8; 64 * 64];
            let found = read_barcodes_in_luma(blank, 64, 64, &catalog::options(&request));
            assert!(found.is_ok_and(|codes| codes.is_empty()), "{kind:?}");
        }
    }

    #[test]
    fn large_images_are_read_at_most_at_sixteen_megapixels() {
        assert_eq!(catalog::fitted_size(6000, 4000), Some((4898, 3265)));
        assert_eq!(catalog::fitted_size(20_000, 100), None);
        assert_eq!(catalog::fitted_size(4000, 4000), None);
        let (width, height) = catalog::fitted_size(8064, 6048).unwrap();
        assert!(u64::from(width) * u64::from(height) <= 16_000_000);
        assert!((f64::from(width) / f64::from(height) - 4.0 / 3.0).abs() < 0.001);
    }
}
