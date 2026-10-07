//! Compare preprocessing and lock placement with one shared ONNX session per encoder.
//!
//! Usage: embedding_pipeline_bench <native|gemma2> <asset-directory>
//!        <locked|prepared|fresh|reuse> <workers> <iterations> [text|image|audio|mixed]
//! Build with `--features local-ml`. Run each case in a separate process under
//! `/usr/bin/time -l` on macOS or `/usr/bin/time -v` on Linux to measure peak RSS.
//! `locked` reproduces preprocessing under the inference lock; `prepared` moves it
//! outside and admits at most two requests. Native `fresh` and `reuse` isolate tensor
//! allocation without inference. This is a controlled provider benchmark, not a runtime
//! scheduler benchmark. Model loading and three warmups are excluded from request times.
//! Both strategies retain the current model state, so their peak RSS cannot establish
//! the memory change from a previous library version.
//! The harness uses standard thread mutexes; its latency percentiles do not describe
//! the production runtime's FIFO scheduling.

#[cfg(not(feature = "local-ml"))]
fn main() {
    eprintln!("Build embedding_pipeline_bench with --features local-ml");
    std::process::exit(2);
}

#[cfg(feature = "local-ml")]
fn main() -> anyhow::Result<()> {
    bench::run()
}

#[cfg(feature = "local-ml")]
mod bench {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Barrier, Condvar, Mutex},
        time::Instant,
    };

    use anyhow::{Context, Result, anyhow, ensure};
    use flow_like_model_provider::embedding::{
        gemma2::{Gemma2Embedding, Gemma2Options},
        interface::{AudioInput, EmbeddingInput, EmbeddingPart, EmbeddingPurpose},
        native::{
            NativeTextEmbedding, NativeTextPreprocessor, Pooling, SessionOptions, TokenizedBatch,
            TokenizerFiles,
        },
    };
    use serde_json::{Value, json};

    const WARMUPS: usize = 3;
    const MAX_PREPARED: usize = 2;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Strategy {
        Locked,
        Prepared,
        Fresh,
        Reuse,
    }

    struct Settings {
        adapter: String,
        directory: PathBuf,
        strategy: Strategy,
        workers: usize,
        iterations: usize,
        workload: String,
    }

    #[derive(Default)]
    struct Sample {
        total_ms: f64,
        gate_ms: f64,
        preprocessing_ms: f64,
        lock_wait_ms: f64,
        inference_ms: f64,
        lock_held_ms: f64,
    }

    /// Bound prepared tensor memory without creating another inference session.
    #[derive(Default)]
    struct Gate {
        active: Mutex<usize>,
        available: Condvar,
    }

    impl Gate {
        fn acquire(&self) -> Permit<'_> {
            let mut active = self.active.lock().unwrap();
            while *active == MAX_PREPARED {
                active = self.available.wait(active).unwrap();
            }
            *active += 1;
            Permit(self)
        }
    }

    struct Permit<'a>(&'a Gate);

    impl Drop for Permit<'_> {
        fn drop(&mut self) {
            *self.0.active.lock().unwrap() -= 1;
            self.0.available.notify_one();
        }
    }

    pub fn run() -> Result<()> {
        let mut args = std::env::args().skip(1);
        let adapter = args.next().context("Missing adapter: native or gemma2")?;
        let directory = args.next().context("Missing model asset directory")?.into();
        let strategy = match args.next().as_deref() {
            Some("locked") => Strategy::Locked,
            Some("prepared") => Strategy::Prepared,
            Some("fresh") => Strategy::Fresh,
            Some("reuse") => Strategy::Reuse,
            _ => return Err(anyhow!("Strategy must be locked, prepared, fresh or reuse")),
        };
        let workers = args.next().context("Missing workers")?.parse()?;
        let iterations = args.next().context("Missing iterations")?.parse()?;
        let workload = args.next().unwrap_or_else(|| "text".into());
        ensure!(args.next().is_none(), "Unexpected extra argument");
        ensure!((1..=32).contains(&workers), "Workers must be 1..=32");
        ensure!(iterations > 0, "Iterations must be positive");
        let settings = Settings {
            adapter,
            directory,
            strategy,
            workers,
            iterations,
            workload,
        };
        match settings.adapter.as_str() {
            "native" => native(&settings),
            "gemma2" => gemma2(&settings),
            _ => Err(anyhow!("Adapter must be native or gemma2")),
        }
    }

    fn ms(start: Instant) -> f64 {
        start.elapsed().as_secs_f64() * 1000.0
    }

    fn native(settings: &Settings) -> Result<()> {
        ensure!(settings.workload == "text", "Native workload must be text");
        let load = Instant::now();
        let bit: Value =
            serde_json::from_slice(&std::fs::read(settings.directory.join("bit.json"))?)?;
        let read = |name| std::fs::read(settings.directory.join(name));
        let max_tokens = bit["parameters"]["input_length"]
            .as_u64()
            .context("Bit has no input_length")?
            .min(8192) as usize;
        let mut model = NativeTextEmbedding::new_from_file(
            settings.directory.join("model.onnx"),
            TokenizerFiles {
                tokenizer_file: read("tokenizer.json")?,
                config_file: read("config.json")?,
                tokenizer_config_file: read("tokenizer_config.json")?,
                special_tokens_map_file: read("special_tokens_map.json")?,
            },
            max_tokens,
            if bit["parameters"]["pooling"] == "CLS" {
                Pooling::Cls
            } else {
                Pooling::Mean
            },
            SessionOptions::default(),
        )?;
        let load_ms = ms(load);
        let preprocessor = model.preprocessor();
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/embedding_parity/manifest.json"
        ))?;
        let prefix = bit["parameters"]["prefix"]["query"].as_str().unwrap_or("");
        let texts = fixture["inputs"]
            .as_array()
            .context("Missing benchmark text inputs")?
            .iter()
            .map(|text| {
                Ok(format!(
                    "{prefix}{}",
                    text.as_str().context("Invalid text")?
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let old = old_tokenize(&preprocessor, &texts, max_tokens)?;
        let full_encodings = || {
            texts
                .iter()
                .map(|text| preprocessor.encode_untruncated_one(text))
                .collect::<Result<Vec<_>>>()
        };
        let new = preprocessor.pad_encodings(full_encodings()?)?;
        ensure!(
            old == new,
            "Single-pass token tensors differ from legacy tensors"
        );
        let reference = model.embed_tokens(&old)?;
        let model = Mutex::new(model);
        let gate = Gate::default();
        let execute = |workspace: &mut TokenizedBatch| -> Result<(Sample, Vec<Vec<f32>>)> {
            let total = Instant::now();
            let mut sample = Sample::default();
            let result = match settings.strategy {
                Strategy::Locked => {
                    let waiting = Instant::now();
                    let mut model = model.lock().unwrap();
                    sample.lock_wait_ms = ms(waiting);
                    let held = Instant::now();
                    let preparation = Instant::now();
                    let tokens = old_tokenize(&preprocessor, &texts, max_tokens)?;
                    sample.preprocessing_ms = ms(preparation);
                    let inference = Instant::now();
                    let vectors = model.embed_tokens(&tokens)?;
                    sample.inference_ms = ms(inference);
                    sample.lock_held_ms = ms(held);
                    vectors
                }
                Strategy::Prepared => {
                    let admission = Instant::now();
                    let _permit = gate.acquire();
                    sample.gate_ms = ms(admission);
                    let preparation = Instant::now();
                    let encodings = full_encodings()?;
                    preprocessor.pad_encodings_into(encodings, workspace)?;
                    sample.preprocessing_ms = ms(preparation);
                    let waiting = Instant::now();
                    let mut model = model.lock().unwrap();
                    sample.lock_wait_ms = ms(waiting);
                    let inference = Instant::now();
                    let vectors = model.embed_tokens(workspace)?;
                    sample.inference_ms = ms(inference);
                    sample.lock_held_ms = sample.inference_ms;
                    vectors
                }
                Strategy::Fresh | Strategy::Reuse => {
                    let preparation = Instant::now();
                    if settings.strategy == Strategy::Fresh {
                        *workspace = preprocessor.tokenize(&texts)?;
                    } else {
                        preprocessor.tokenize_into(&texts, workspace)?;
                    }
                    std::hint::black_box(&*workspace);
                    sample.preprocessing_ms = ms(preparation);
                    Vec::new()
                }
            };
            sample.total_ms = ms(total);
            Ok((sample, result))
        };
        let (samples, elapsed_ms, distance) = measure(settings, &reference, execute)?;
        report(
            settings,
            samples,
            elapsed_ms,
            load_ms,
            texts.len(),
            distance,
            json!({"bit":bit["id"], "token_shape":old.input_ids.shape(), "token_tensors_equal":true}),
        );
        Ok(())
    }

    /// The previous recipe path cloned an untruncated tokenizer for validation,
    /// encoded each input, then encoded the batch again for inference.
    fn old_tokenize(
        preprocessor: &NativeTextPreprocessor,
        texts: &[String],
        max_tokens: usize,
    ) -> Result<TokenizedBatch> {
        let mut validation = preprocessor.tokenizer().clone();
        validation
            .with_truncation(None)
            .map_err(|error| anyhow!(error.to_string()))?;
        validation.with_padding(None);
        for text in texts {
            let encoding = validation
                .encode(text.as_str(), true)
                .map_err(|error| anyhow!(error.to_string()))?;
            ensure!(
                encoding.len() <= max_tokens,
                "Benchmark input exceeds context"
            );
        }
        preprocessor.tokenize(texts)
    }

    fn gemma2(settings: &Settings) -> Result<()> {
        ensure!(
            matches!(settings.strategy, Strategy::Locked | Strategy::Prepared),
            "Gemma2 supports locked and prepared strategies"
        );
        let load = Instant::now();
        let mut model = Gemma2Embedding::load(&settings.directory, Gemma2Options::default())?;
        let load_ms = ms(load);
        let preprocessor = model.preprocessor();
        let input = media_input(&settings.workload)?;
        let purpose = EmbeddingPurpose::Document;
        let reference = model.embed(std::slice::from_ref(&input), purpose)?;
        let equivalent = model.embed_prepared(preprocessor.prepare(&input, purpose)?)?;
        ensure!(
            vector_distance(&reference, &[equivalent])? <= 1e-7,
            "Prepared Gemma2 output differs from synchronous output"
        );
        let model = Mutex::new(model);
        let gate = Gate::default();
        let execute = |_: &mut TokenizedBatch| -> Result<(Sample, Vec<Vec<f32>>)> {
            let total = Instant::now();
            let mut sample = Sample::default();
            let result = if settings.strategy == Strategy::Locked {
                let waiting = Instant::now();
                let mut model = model.lock().unwrap();
                sample.lock_wait_ms = ms(waiting);
                let held = Instant::now();
                let preparation = Instant::now();
                let prepared = preprocessor.prepare(&input, purpose)?;
                sample.preprocessing_ms = ms(preparation);
                let inference = Instant::now();
                let vector = model.embed_prepared(prepared)?;
                sample.inference_ms = ms(inference);
                sample.lock_held_ms = ms(held);
                vector
            } else {
                let admission = Instant::now();
                let _permit = gate.acquire();
                sample.gate_ms = ms(admission);
                let preparation = Instant::now();
                let prepared = preprocessor.prepare(&input, purpose)?;
                sample.preprocessing_ms = ms(preparation);
                let waiting = Instant::now();
                let mut model = model.lock().unwrap();
                sample.lock_wait_ms = ms(waiting);
                let inference = Instant::now();
                let vector = model.embed_prepared(prepared)?;
                sample.inference_ms = ms(inference);
                sample.lock_held_ms = sample.inference_ms;
                vector
            };
            sample.total_ms = ms(total);
            Ok((sample, vec![result]))
        };
        let (samples, elapsed_ms, distance) = measure(settings, &reference, execute)?;
        report(
            settings,
            samples,
            elapsed_ms,
            load_ms,
            1,
            distance,
            json!({
                "dimensions":768,
                "image_pixels":matches!(settings.workload.as_str(),"image"|"mixed").then_some([640,480]),
                "audio_seconds":matches!(settings.workload.as_str(),"audio"|"mixed").then_some(1),
                "audio_sample_rate":matches!(settings.workload.as_str(),"audio"|"mixed").then_some(16000)
            }),
        );
        Ok(())
    }

    fn media_input(workload: &str) -> Result<EmbeddingInput> {
        let image = || {
            EmbeddingPart::Image(Arc::new(image::DynamicImage::ImageRgb8(
                image::RgbImage::from_fn(640, 480, |x, y| {
                    image::Rgb([
                        ((x * 3 + y) % 256) as u8,
                        ((x + y * 5) % 256) as u8,
                        ((x * 7 + y * 11) % 256) as u8,
                    ])
                }),
            )))
        };
        let audio = || {
            EmbeddingPart::Audio(AudioInput {
                samples: (0..16000)
                    .map(|i| (i as f32 * std::f32::consts::TAU * 440.0 / 16000.0).sin() * 0.125)
                    .collect::<Vec<_>>()
                    .into(),
                sample_rate: 16000,
                channels: 1,
            })
        };
        let text = || EmbeddingPart::Text("A colorful image and a steady musical tone.".into());
        let parts = match workload {
            "text" => vec![text()],
            "image" => vec![image()],
            "audio" => vec![audio()],
            "mixed" => vec![text(), image(), audio()],
            _ => return Err(anyhow!("Workload must be text, image, audio or mixed")),
        };
        Ok(EmbeddingInput { parts, title: None })
    }

    fn measure(
        settings: &Settings,
        reference: &[Vec<f32>],
        execute: impl Fn(&mut TokenizedBatch) -> Result<(Sample, Vec<Vec<f32>>)> + Sync,
    ) -> Result<(Vec<Sample>, f64, Option<f64>)> {
        let inference = matches!(settings.strategy, Strategy::Locked | Strategy::Prepared);
        let mut warmup = TokenizedBatch::default();
        for _ in 0..WARMUPS {
            let (_, vectors) = execute(&mut warmup)?;
            if inference {
                ensure!(
                    vector_distance(reference, &vectors)? <= 1e-7,
                    "Warmup vector drift"
                );
            }
        }
        // All workers warm their own reusable tensor buffers before the timed barrier.
        let barrier = Barrier::new(settings.workers + 1);
        std::thread::scope(|scope| {
            let workers = (0..settings.workers)
                .map(|_| {
                    scope.spawn(|| -> Result<(Vec<Sample>, f64)> {
                        let mut workspace = TokenizedBatch::default();
                        let readiness = execute(&mut workspace);
                        barrier.wait();
                        barrier.wait();
                        readiness?;
                        let mut samples = Vec::with_capacity(settings.iterations);
                        let mut distance = 0.0f64;
                        for _ in 0..settings.iterations {
                            let (sample, vectors) = execute(&mut workspace)?;
                            if inference {
                                distance = distance.max(vector_distance(reference, &vectors)?);
                            }
                            samples.push(sample);
                        }
                        Ok((samples, distance))
                    })
                })
                .collect::<Vec<_>>();
            barrier.wait();
            let start = Instant::now();
            barrier.wait();
            let mut samples = Vec::with_capacity(settings.workers * settings.iterations);
            let mut distance = 0.0f64;
            for worker in workers {
                let (mut output, worker_distance) = worker
                    .join()
                    .map_err(|_| anyhow!("Benchmark worker panicked"))??;
                samples.append(&mut output);
                distance = distance.max(worker_distance);
            }
            let elapsed_ms = ms(start);
            ensure!(distance <= 1e-7, "Benchmark vector drift: {distance}");
            Ok((samples, elapsed_ms, inference.then_some(distance)))
        })
    }

    fn vector_distance(reference: &[Vec<f32>], actual: &[Vec<f32>]) -> Result<f64> {
        ensure!(reference.len() == actual.len(), "Vector count changed");
        let mut maximum = 0.0f64;
        for (left, right) in reference.iter().zip(actual) {
            ensure!(
                !right.is_empty() && left.len() == right.len(),
                "Vector shape changed"
            );
            ensure!(right.iter().all(|v| v.is_finite()), "Non-finite output");
            let dot = left
                .iter()
                .zip(right)
                .map(|(&a, &b)| a as f64 * b as f64)
                .sum::<f64>();
            let norm = |values: &[f32]| {
                values
                    .iter()
                    .map(|&v| (v as f64).powi(2))
                    .sum::<f64>()
                    .sqrt()
            };
            let denominator = norm(left) * norm(right);
            ensure!(denominator > 0.0, "Zero embedding");
            maximum = maximum.max((1.0 - dot / denominator).abs());
        }
        Ok(maximum)
    }

    fn stats(samples: &[Sample], field: impl Fn(&Sample) -> f64) -> Value {
        let mut values = samples.iter().map(field).collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        let percentile =
            |p: f64| values[((p * values.len() as f64).ceil() as usize).saturating_sub(1)];
        json!({"median":percentile(0.5), "p95":percentile(0.95),
            "mean":values.iter().sum::<f64>() / values.len() as f64})
    }

    #[allow(clippy::too_many_arguments)]
    fn report(
        settings: &Settings,
        samples: Vec<Sample>,
        elapsed_ms: f64,
        load_ms: f64,
        items_per_request: usize,
        distance: Option<f64>,
        workload: Value,
    ) {
        println!(
            "{}",
            json!({
                "benchmark":"embedding_pipeline_v1",
                "adapter":settings.adapter, "assets":Path::new(&settings.directory),
                "strategy":format!("{:?}",settings.strategy).to_lowercase(),
                "workload":settings.workload, "workload_details":workload,
                "workers":settings.workers, "iterations_per_worker":settings.iterations,
                "requests":samples.len(), "items_per_request":items_per_request,
                "warmups":WARMUPS, "worker_warmups":1,
                "prepared_request_limit":(settings.strategy == Strategy::Prepared).then_some(MAX_PREPARED),
                "load_ms":load_ms, "elapsed_ms":elapsed_ms,
                "requests_per_second":samples.len() as f64 * 1000.0 / elapsed_ms,
                "items_per_second":(samples.len()*items_per_request) as f64 * 1000.0 / elapsed_ms,
                "total_ms":stats(&samples,|s|s.total_ms),
                "admission_wait_ms":stats(&samples,|s|s.gate_ms),
                "preprocessing_ms":stats(&samples,|s|s.preprocessing_ms),
                "inference_lock_wait_ms":stats(&samples,|s|s.lock_wait_ms),
                "inference_ms":stats(&samples,|s|s.inference_ms),
                "inference_lock_held_ms":stats(&samples,|s|s.lock_held_ms),
                "max_cosine_distance":distance,
                "throughput_includes_output_validation":true,
            "provider_workflow_comparison":true,
            "rss_is_current_model_state":true,
                "available_parallelism":std::thread::available_parallelism().map(usize::from).ok(),
                "os":std::env::consts::OS, "architecture":std::env::consts::ARCH,
            })
        );
    }
}
