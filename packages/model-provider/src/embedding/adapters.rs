//! Compatibility adapters preserve the established text and image pipeline contracts.

use std::sync::Arc;

use async_trait::async_trait;

use super::{EmbeddingModelLogic, interface::*};
use crate::image_embedding::ImageEmbeddingModelLogic;

pub struct LegacyEmbeddingAdapter {
    descriptor: EmbeddingDescriptor,
    text: Option<Arc<dyn EmbeddingModelLogic>>,
    image: Option<Arc<dyn ImageEmbeddingModelLogic>>,
}

impl LegacyEmbeddingAdapter {
    pub fn text(model: Arc<dyn EmbeddingModelLogic>, descriptor: EmbeddingDescriptor) -> Self {
        Self {
            descriptor,
            text: Some(model),
            image: None,
        }
    }

    pub fn image(
        model: Arc<dyn ImageEmbeddingModelLogic>,
        descriptor: EmbeddingDescriptor,
    ) -> Self {
        Self {
            descriptor,
            text: None,
            image: Some(model),
        }
    }
}

#[async_trait]
impl EmbeddingModel for LegacyEmbeddingAdapter {
    fn descriptor(&self) -> &EmbeddingDescriptor {
        &self.descriptor
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingBatch, EmbeddingError> {
        self.descriptor.validate(&request)?;
        let mut texts = Vec::new();
        let mut text_indices = Vec::new();
        let mut images = Vec::new();
        let mut image_indices = Vec::new();
        for (index, item) in request.items.iter().enumerate() {
            if item.parts.len() != 1 || item.title.is_some() {
                return Err(EmbeddingError::InvalidInput(
                    "This legacy model accepts one content part per item without a title".into(),
                ));
            }
            match &item.parts[0] {
                EmbeddingPart::Text(text) => {
                    texts.push(text.clone());
                    text_indices.push(index);
                }
                EmbeddingPart::Image(image) if self.image.is_some() => {
                    images.push(image.as_ref().clone());
                    image_indices.push(index);
                }
                part => return Err(EmbeddingError::UnsupportedModality(part.modality())),
            }
        }
        let mut vectors = vec![Vec::new(); request.items.len()];
        if !texts.is_empty() {
            let result = match (&self.text, &self.image, request.purpose) {
                (Some(model), _, EmbeddingPurpose::Query) => model.text_embed_query(&texts).await,
                (Some(model), _, EmbeddingPurpose::Document) => {
                    model.text_embed_document(&texts).await
                }
                (_, Some(model), EmbeddingPurpose::Query) => model.text_embed_query(&texts).await,
                (_, Some(model), EmbeddingPurpose::Document) => {
                    model.text_embed_document(&texts).await
                }
                _ => return Err(EmbeddingError::UnsupportedPurpose(request.purpose)),
            }
            .map_err(EmbeddingError::Backend)?;
            place_vectors(&mut vectors, &text_indices, result)?;
        }
        if !images.is_empty() {
            let model = self
                .image
                .as_ref()
                .ok_or(EmbeddingError::UnsupportedModality(
                    EmbeddingModality::Image,
                ))?;
            let result = model
                .image_embed(images)
                .await
                .map_err(EmbeddingError::Backend)?;
            place_vectors(&mut vectors, &image_indices, result)?;
        }
        self.descriptor.finish(&request, vectors)
    }
}

fn place_vectors(
    destination: &mut [Vec<f32>],
    indices: &[usize],
    vectors: Vec<Vec<f32>>,
) -> Result<(), EmbeddingError> {
    if indices.len() != vectors.len() {
        return Err(EmbeddingError::InvalidOutput(
            "Embedding backend returned the wrong number of vectors".into(),
        ));
    }
    for (&index, vector) in indices.iter().zip(vectors) {
        destination[index] = vector;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::GeneralTextSplitter;
    use flow_like_types_contracts::Cacheable;
    use std::{any::Any, sync::Mutex};

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Query(Vec<String>),
        Document(Vec<String>),
        Images(Vec<u32>),
    }

    #[derive(Clone, Default)]
    struct RecordingModel {
        calls: Arc<Mutex<Vec<Call>>>,
        omit_last: bool,
    }

    impl RecordingModel {
        fn vectors(&self, texts: &[String], document: bool) -> Vec<Vec<f32>> {
            self.calls.lock().unwrap().push(if document {
                Call::Document(texts.to_vec())
            } else {
                Call::Query(texts.to_vec())
            });
            let mut vectors: Vec<Vec<f32>> = texts
                .iter()
                .map(|text| vec![if document { -1.0 } else { 1.0 }, text.len() as f32])
                .collect();
            if self.omit_last {
                vectors.pop();
            }
            vectors
        }
    }

    impl Cacheable for RecordingModel {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    #[async_trait]
    impl EmbeddingModelLogic for RecordingModel {
        async fn get_splitter(
            &self,
            _: Option<usize>,
            _: Option<usize>,
        ) -> anyhow::Result<(GeneralTextSplitter, GeneralTextSplitter)> {
            anyhow::bail!("This routing fixture does not split text")
        }
        async fn text_embed_query(&self, texts: &Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(self.vectors(texts, false))
        }
        async fn text_embed_document(&self, texts: &Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(self.vectors(texts, true))
        }
        fn as_cacheable(&self) -> Arc<dyn Cacheable> {
            Arc::new(self.clone())
        }
    }

    #[async_trait]
    impl ImageEmbeddingModelLogic for RecordingModel {
        async fn get_splitter(
            &self,
            _: Option<usize>,
            _: Option<usize>,
        ) -> anyhow::Result<(GeneralTextSplitter, GeneralTextSplitter)> {
            anyhow::bail!("This routing fixture does not split text")
        }
        async fn text_embed_query(&self, texts: &Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(self.vectors(texts, false))
        }
        async fn text_embed_document(&self, texts: &Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(self.vectors(texts, true))
        }
        async fn image_embed(
            &self,
            images: Vec<image::DynamicImage>,
        ) -> anyhow::Result<Vec<Vec<f32>>> {
            self.calls.lock().unwrap().push(Call::Images(
                images.iter().map(image::DynamicImage::width).collect(),
            ));
            Ok(images
                .iter()
                .map(|image| vec![image.width() as f32, 2.0])
                .collect())
        }
        fn as_cacheable(&self) -> Arc<dyn Cacheable> {
            Arc::new(self.clone())
        }
    }

    fn descriptor(image: bool) -> EmbeddingDescriptor {
        EmbeddingDescriptor {
            model_id: "legacy-model".into(),
            adapter: "legacy".into(),
            modalities: if image {
                vec![EmbeddingModality::Text, EmbeddingModality::Image]
            } else {
                vec![EmbeddingModality::Text]
            },
            joint_combinations: vec![],
            purposes: vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document],
            space: EmbeddingSpace {
                id: "legacy-space".into(),
                dimensions: 2,
                normalized: false,
                metric: EmbeddingMetric::Cosine,
            },
            supported_dimensions: vec![2],
            limits: EmbeddingLimits::default(),
            pipeline_fingerprint: "legacy-revision".into(),
        }
    }

    #[tokio::test]
    async fn query_and_document_forward_raw_text_and_preserve_legacy_vectors() {
        let backend = Arc::new(RecordingModel::default());
        let adapter = LegacyEmbeddingAdapter::text(backend.clone(), descriptor(false));
        let raw = vec![
            "  raw query  ".to_string(),
            "query: already written".to_string(),
        ];
        for purpose in [EmbeddingPurpose::Query, EmbeddingPurpose::Document] {
            let output = adapter
                .embed(EmbeddingRequest::texts(raw.clone(), purpose))
                .await
                .unwrap();
            let expected: Vec<Vec<f32>> = raw
                .iter()
                .map(|text| {
                    vec![
                        if purpose == EmbeddingPurpose::Query {
                            1.0
                        } else {
                            -1.0
                        },
                        text.len() as f32,
                    ]
                })
                .collect();
            assert_eq!(output.embeddings, expected);
            assert_eq!(output.provenance.pipeline_fingerprint, "legacy-revision");
        }
        assert_eq!(
            *backend.calls.lock().unwrap(),
            vec![Call::Query(raw.clone()), Call::Document(raw)]
        );
    }

    #[tokio::test]
    async fn separate_text_and_image_items_keep_their_original_order() {
        let backend = Arc::new(RecordingModel::default());
        let adapter = LegacyEmbeddingAdapter::image(backend.clone(), descriptor(true));
        let request = EmbeddingRequest {
            items: vec![
                EmbeddingInput::image(image::DynamicImage::new_rgb8(3, 1)),
                EmbeddingInput::text("abc"),
                EmbeddingInput::image(image::DynamicImage::new_rgb8(7, 1)),
                EmbeddingInput::text("hello"),
            ],
            purpose: EmbeddingPurpose::Query,
            options: EmbeddingOptions::default(),
        };
        let output = adapter.embed(request).await.unwrap();
        assert_eq!(
            output.embeddings,
            vec![
                vec![3.0, 2.0],
                vec![1.0, 3.0],
                vec![7.0, 2.0],
                vec![1.0, 5.0]
            ]
        );
        assert_eq!(
            *backend.calls.lock().unwrap(),
            vec![
                Call::Query(vec!["abc".into(), "hello".into()]),
                Call::Images(vec![3, 7])
            ]
        );
    }

    #[tokio::test]
    async fn unsupported_joint_content_titles_and_purposes_never_reach_the_backend() {
        let backend = Arc::new(RecordingModel::default());
        let adapter = LegacyEmbeddingAdapter::image(backend.clone(), descriptor(true));
        let mut request = EmbeddingRequest::texts(["caption".into()], EmbeddingPurpose::Document);
        request.items[0].parts.push(EmbeddingPart::Image(Arc::new(
            image::DynamicImage::new_rgb8(2, 2),
        )));
        assert!(matches!(
            adapter.embed(request).await,
            Err(EmbeddingError::UnsupportedCombination(_))
        ));
        let mut request = EmbeddingRequest::texts(["caption".into()], EmbeddingPurpose::Document);
        request.items[0].title = Some("A title".into());
        assert!(matches!(
            adapter.embed(request).await,
            Err(EmbeddingError::InvalidInput(_))
        ));
        let mut request = EmbeddingRequest::texts(["first".into()], EmbeddingPurpose::Document);
        request.items[0]
            .parts
            .push(EmbeddingPart::Text("second".into()));
        assert!(matches!(
            adapter.embed(request).await,
            Err(EmbeddingError::InvalidInput(_))
        ));
        let request = EmbeddingRequest::texts(["caption".into()], EmbeddingPurpose::Classification);
        assert!(matches!(
            adapter.embed(request).await,
            Err(EmbeddingError::UnsupportedPurpose(
                EmbeddingPurpose::Classification
            ))
        ));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn wrong_backend_cardinality_is_reported_without_reordering_or_padding() {
        let backend = Arc::new(RecordingModel {
            omit_last: true,
            ..Default::default()
        });
        let adapter = LegacyEmbeddingAdapter::text(backend, descriptor(false));
        let request =
            EmbeddingRequest::texts(["one".into(), "two".into()], EmbeddingPurpose::Query);
        assert!(matches!(
            adapter.embed(request).await,
            Err(EmbeddingError::InvalidOutput(_))
        ));
    }

    #[tokio::test]
    async fn an_empty_batch_returns_no_vectors_without_a_backend_request() {
        let backend = Arc::new(RecordingModel::default());
        let adapter = LegacyEmbeddingAdapter::text(backend.clone(), descriptor(false));
        let output = adapter
            .embed(EmbeddingRequest::texts([], EmbeddingPurpose::Query))
            .await
            .unwrap();
        assert!(output.embeddings.is_empty());
        assert_eq!(output.usage.input_items, 0);
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    // Uses the same repository service credentials as the API's ignored Internal
    // embedding tests. Set FLOW_LIKE_CAPTURE_REMOTE_EMBEDDINGS=1 to refresh the
    // numeric fixtures after reviewing a deliberate upstream model change.
    #[tokio::test]
    #[ignore = "Calls the configured INTERNAL_EMBEDDING_ENDPOINT with INTERNAL_EMBEDDING_SECRET"]
    async fn published_internal_models_match_legacy_and_golden_vectors() -> anyhow::Result<()> {
        use crate::{
            embedding::endpoint::EndpointEmbeddingModel, provider::EmbeddingModelProvider,
        };
        use std::{collections::HashMap, path::Path, time::Duration};

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut configuration = HashMap::new();
        // The iterator keeps credentials out of the process-wide test environment.
        #[allow(deprecated)]
        if let Ok(entries) = dotenv::from_path_iter(root.join(".env")) {
            for (name, value) in entries.flatten() {
                if matches!(
                    name.as_str(),
                    "INTERNAL_EMBEDDING_ENDPOINT" | "INTERNAL_EMBEDDING_SECRET"
                ) {
                    configuration.insert(name, value);
                }
            }
        }
        for name in ["INTERNAL_EMBEDDING_ENDPOINT", "INTERNAL_EMBEDDING_SECRET"] {
            if let Ok(value) = std::env::var(name) {
                configuration.insert(name.into(), value);
            }
            anyhow::ensure!(
                configuration
                    .get(name)
                    .is_some_and(|value| !value.trim().is_empty()),
                "Missing Internal embedding service test configuration"
            );
        }
        let endpoint = configuration["INTERNAL_EMBEDDING_ENDPOINT"]
            .trim()
            .trim_end_matches('/');
        let base = if endpoint.ends_with("/v1/embeddings") {
            endpoint.trim_end_matches("/embeddings").to_owned()
        } else if endpoint.ends_with("/v1") {
            endpoint.to_owned()
        } else {
            format!("{endpoint}/v1")
        };
        let capture = std::env::var("FLOW_LIKE_CAPTURE_REMOTE_EMBEDDINGS").as_deref() == Ok("1");
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/embedding_remote_parity");
        let inputs = vec![
            "A small cat sits beside a sunny window.".to_owned(),
            "Database indexes make relevant records easier to retrieve.".to_owned(),
            "How can renewable energy reduce industrial emissions?".to_owned(),
        ];
        let published: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/embedding_parity/manifest.json"
        ))?;
        let golden: Option<serde_json::Value> = if capture {
            None
        } else {
            Some(serde_json::from_slice(&std::fs::read(
                fixtures.join("manifest.json"),
            )?)?)
        };
        if let Some(golden) = &golden {
            assert_eq!(golden["inputs"], serde_json::json!(inputs));
        }
        let mut models = Vec::new();
        let mut captured = Vec::new();
        let mut comparisons = 0;
        let mut golden_comparisons = 0;
        let mut maximum = 0.0_f64;
        for entry in published["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["status"] == "remote_only")
        {
            let bit = &entry["bit"];
            let id = bit["id"].as_str().unwrap();
            let provider: EmbeddingModelProvider =
                serde_json::from_value(bit["parameters"].clone())?;
            let model_id = provider
                .remote
                .as_ref()
                .and_then(|remote| remote.model_id.as_deref())
                .unwrap();
            let dimensions = provider.vector_length as usize;
            let legacy = Arc::new(
                EndpointEmbeddingModel::new(
                    &base,
                    &configuration["INTERNAL_EMBEDDING_SECRET"],
                    model_id,
                    provider.clone(),
                )
                .map_err(|_| {
                    anyhow::anyhow!("Could not construct Internal embedding test client")
                })?,
            );
            let mut description = descriptor(false);
            description.model_id = id.into();
            description.space.dimensions = dimensions;
            description.space.normalized = false;
            description.supported_dimensions = vec![dimensions];
            let adapter = LegacyEmbeddingAdapter::text(legacy.clone(), description);
            let golden_vectors = if golden.is_some() {
                Some(
                    std::fs::read(fixtures.join(format!("{id}.f32")))?
                        .chunks_exact(4)
                        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            };
            let mut bytes = Vec::new();
            let mut cases = Vec::new();
            for (name, purpose) in [
                ("query", EmbeddingPurpose::Query),
                ("document", EmbeddingPurpose::Document),
            ] {
                let reference = tokio::time::timeout(Duration::from_secs(180), async {
                    if purpose == EmbeddingPurpose::Query {
                        legacy.text_embed_query(&inputs).await
                    } else {
                        legacy.text_embed_document(&inputs).await
                    }
                }).await.map_err(|_| anyhow::anyhow!("Internal embedding baseline timed out for {id}"))?
                    .map_err(|_| anyhow::anyhow!("Internal embedding baseline request failed for {id}; endpoint details suppressed"))?;
                let modern = tokio::time::timeout(Duration::from_secs(180), adapter.embed(
                    EmbeddingRequest::texts(inputs.clone(), purpose)
                )).await.map_err(|_| anyhow::anyhow!("Internal embedding adapter timed out for {id}"))?
                    .map_err(|_| anyhow::anyhow!("Internal embedding adapter request failed for {id}; endpoint details suppressed"))?;
                assert_eq!(reference.len(), inputs.len());
                assert_eq!(modern.embeddings.len(), inputs.len());
                let offset = bytes.len() / 4;
                for (row, (reference, actual)) in
                    reference.iter().zip(&modern.embeddings).enumerate()
                {
                    assert_eq!(reference.len(), dimensions);
                    assert_eq!(actual.len(), dimensions);
                    let distance = remote_cosine_distance(reference, actual);
                    assert!(
                        distance <= 1e-5,
                        "{id} {name} row{row}: cosine distance {distance}"
                    );
                    maximum = maximum.max(distance);
                    comparisons += 1;
                    if let Some(golden) = &golden_vectors {
                        let start = offset + row * dimensions;
                        let expected = &golden[start..start + dimensions];
                        let legacy_distance = remote_cosine_distance(reference, expected);
                        let modern_distance = remote_cosine_distance(actual, expected);
                        assert!(
                            legacy_distance <= 1e-5,
                            "{id} {name} legacy service output changed"
                        );
                        assert!(
                            modern_distance <= 1e-5,
                            "{id} {name} adapter differs from captured service output"
                        );
                        maximum = maximum.max(legacy_distance).max(modern_distance);
                        golden_comparisons += 2;
                    }
                    bytes.extend(reference.iter().flat_map(|value| value.to_le_bytes()));
                }
                cases.push(serde_json::json!({"name":name,"rows":inputs.len(),"dimensions":dimensions,"offset_floats":offset}));
            }
            if let Some(golden) = &golden_vectors {
                assert_eq!(golden.len() * 4, bytes.len());
            }
            models.push(serde_json::json!({"id":id,"model":model_id,"parameters":bit["parameters"],"vector_file":format!("{id}.f32"),"cases":cases}));
            captured.push((format!("{id}.f32"), bytes));
            eprintln!("{id}: legacy and modern Internal service vectors matched");
        }
        assert_eq!(models.len(), 3);
        if capture {
            std::fs::create_dir_all(&fixtures)?;
            for (name, bytes) in captured {
                std::fs::write(fixtures.join(name), bytes)?;
            }
            let manifest = serde_json::json!({
                "schema_version":1,
                "reference":"Unchanged EndpointEmbeddingModel with published Bit query/document prefixes",
                "reference_source_blake3":blake3::hash(include_bytes!("endpoint.rs")).to_hex().to_string(),
                "captured_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                "inputs":inputs,
                "models":models,
            });
            std::fs::write(
                fixtures.join("manifest.json"),
                serde_json::to_vec_pretty(&manifest)?,
            )?;
        }
        eprintln!(
            "3 hosted models, {comparisons} legacy/modern vector pairs, {golden_comparisons} golden comparisons, max cosine distance {maximum:.3e}"
        );
        Ok(())
    }

    fn remote_cosine_distance(left: &[f32], right: &[f32]) -> f64 {
        assert_eq!(left.len(), right.len());
        let (mut dot, mut left_norm, mut right_norm) = (0.0, 0.0, 0.0);
        for (&left, &right) in left.iter().zip(right) {
            assert!(left.is_finite() && right.is_finite());
            let (left, right) = (f64::from(left), f64::from(right));
            dot += left * right;
            left_norm += left * left;
            right_norm += right * right;
        }
        assert!(left_norm > 0.0 && right_norm > 0.0);
        (1.0 - dot / (left_norm * right_norm).sqrt()).max(0.0)
    }

    #[test]
    fn hosted_golden_fixtures_cover_every_published_remote_bit() -> anyhow::Result<()> {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/embedding_remote_parity");
        let golden: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json"))?)?;
        let published: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/embedding_parity/manifest.json"
        ))?;
        assert_eq!(golden["schema_version"], 1);
        assert!(
            golden["reference"]
                .as_str()
                .unwrap()
                .contains("EndpointEmbeddingModel")
        );
        assert_eq!(
            golden["reference_source_blake3"].as_str().unwrap().len(),
            64
        );
        let mut actual_ids = std::collections::BTreeSet::new();
        let expected_ids = [
            "d310v8j1z1pfhkggv60xbwwq",
            "emamoveivs0ax85tkq4ro90r",
            "puronf9t6j72eti9f12jmenz",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        for model in golden["models"].as_array().unwrap() {
            let id = model["id"].as_str().unwrap();
            assert!(actual_ids.insert(id), "Duplicate hosted fixture");
            let source = published["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["bit"]["id"] == id)
                .unwrap();
            assert_eq!(source["status"], "remote_only");
            assert_eq!(model["parameters"], source["bit"]["parameters"]);
            assert_eq!(model["vector_file"], format!("{id}.f32"));
            let bytes = std::fs::read(directory.join(format!("{id}.f32")))?;
            assert_eq!(bytes.len() % 4, 0);
            let vectors = bytes
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                .collect::<Vec<_>>();
            let mut offset = 0;
            let cases = model["cases"].as_array().unwrap();
            assert_eq!(cases.len(), 2);
            for (case, name) in cases.iter().zip(["query", "document"]) {
                assert_eq!(case["name"], name);
                assert_eq!(case["offset_floats"], offset);
                let dimensions = case["dimensions"].as_u64().unwrap() as usize;
                let rows = case["rows"].as_u64().unwrap() as usize;
                assert_eq!(
                    case["dimensions"],
                    source["bit"]["parameters"]["vector_length"]
                );
                assert_eq!(rows, golden["inputs"].as_array().unwrap().len());
                assert!(dimensions > 0 && rows > 0);
                for _ in 0..rows {
                    let vector = &vectors[offset..offset + dimensions];
                    assert!(vector.iter().all(|value| value.is_finite()));
                    assert!(vector.iter().any(|value| *value != 0.0));
                    offset += dimensions;
                }
            }
            assert_eq!(offset, vectors.len());
        }
        assert_eq!(actual_ids, expected_ids);
        Ok(())
    }

    #[cfg(feature = "remote-ml")]
    #[tokio::test]
    async fn published_remote_bits_keep_the_exact_proxy_request_contract() {
        use crate::{embedding::proxy::ProxyEmbeddingModel, provider::EmbeddingModelProvider};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/embedding_parity/manifest.json"
        ))
        .unwrap();
        let remote_ids = [
            "d310v8j1z1pfhkggv60xbwwq",
            "emamoveivs0ax85tkq4ro90r",
            "puronf9t6j72eti9f12jmenz",
        ];
        for id in remote_ids {
            let bit = manifest["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|model| &model["bit"])
                .find(|bit| bit["id"] == id)
                .unwrap();
            let provider: EmbeddingModelProvider =
                serde_json::from_value(bit["parameters"].clone()).unwrap();
            assert_eq!(provider.provider.provider_name, "Premium");
            assert!(provider.supports_remote());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let dimensions = provider.vector_length as usize;
            let server = tokio::spawn(async move {
                let mut requests = Vec::new();
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let (header_end, content_length) = loop {
                        let mut buffer = [0_u8; 4096];
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0, "request ended before its headers");
                        bytes.extend_from_slice(&buffer[..count]);
                        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            let headers =
                                String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    line.strip_prefix("content-length:")
                                        .map(|value| value.trim().parse::<usize>().unwrap())
                                })
                                .unwrap();
                            break (end + 4, length);
                        }
                    };
                    while bytes.len() < header_end + content_length {
                        let mut buffer = [0_u8; 4096];
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0, "request ended before its JSON body");
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    requests.push((
                        String::from_utf8(bytes[..header_end].to_vec()).unwrap(),
                        bytes[header_end..header_end + content_length].to_vec(),
                    ));
                    let mut vector = vec![0.0_f32; dimensions];
                    vector[0] = 1.0;
                    let response = serde_json::json!({"embeddings":[vector],"model":id,"usage":{"prompt_tokens":3,"total_tokens":3}}).to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                }
                requests
            });
            let proxy = ProxyEmbeddingModel::new(
                provider,
                id.into(),
                "fixture-token".into(),
                vec![("x-flow-like-app-id".into(), "fixture-app".into())],
                endpoint,
            );
            let mut description = descriptor(false);
            description.model_id = id.into();
            description.space.dimensions = dimensions;
            description.supported_dimensions = vec![dimensions];
            let adapter = LegacyEmbeddingAdapter::text(Arc::new(proxy), description);
            for purpose in [EmbeddingPurpose::Query, EmbeddingPurpose::Document] {
                let response = adapter
                    .embed(EmbeddingRequest::texts([" raw input ".into()], purpose))
                    .await
                    .unwrap();
                assert_eq!(response.embeddings.len(), 1);
                assert_eq!(response.embeddings[0].len(), dimensions);
                assert_eq!(response.embeddings[0][0], 1.0);
            }
            let captured = server.await.unwrap();
            for ((headers, body), purpose) in captured.iter().zip(["query", "document"]) {
                assert!(headers.starts_with("POST /api/v1/embeddings/embed HTTP/1.1\r\n"));
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-token\r\n")
                );
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("x-flow-like-app-id: fixture-app\r\n")
                );
                let expected = format!(
                    r#"{{"model":"{id}","input":[" raw input "],"embed_type":"{purpose}"}}"#
                );
                assert_eq!(body.as_slice(), expected.as_bytes(), "published Bit {id}");
            }
        }
    }
}
