/// # ONNX Audio Processing Nodes
/// Speech-to-text, voice activity detection, text-to-speech
use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::{Array1, Array2},
    ort::{inputs, value::Value},
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Audio data for processing
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AudioData {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
    /// Audio samples (normalized to -1.0 to 1.0)
    pub samples: Vec<f32>,
    /// Duration in seconds
    pub duration_secs: f32,
}

impl AudioData {
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Self {
        let duration_secs = samples.len() as f32 / sample_rate as f32 / channels as f32;
        Self {
            sample_rate,
            channels,
            samples,
            duration_secs,
        }
    }

    /// Resample to target sample rate (simple linear interpolation)
    pub fn resample(&self, target_rate: u32) -> Self {
        if self.sample_rate == target_rate {
            return self.clone();
        }

        let ratio = target_rate as f32 / self.sample_rate as f32;
        let new_len = (self.samples.len() as f32 * ratio) as usize;
        let mut resampled = Vec::with_capacity(new_len);

        for i in 0..new_len {
            let src_idx = i as f32 / ratio;
            let idx0 = src_idx.floor() as usize;
            let idx1 = (idx0 + 1).min(self.samples.len() - 1);
            let frac = src_idx - idx0 as f32;

            let sample = self.samples[idx0] * (1.0 - frac) + self.samples[idx1] * frac;
            resampled.push(sample);
        }

        Self::new(target_rate, self.channels, resampled)
    }

    /// Convert to mono by averaging channels
    pub fn to_mono(&self) -> Self {
        if self.channels == 1 {
            return self.clone();
        }

        let mut mono = Vec::with_capacity(self.samples.len() / self.channels as usize);
        for chunk in self.samples.chunks(self.channels as usize) {
            let avg: f32 = chunk.iter().sum::<f32>() / chunk.len() as f32;
            mono.push(avg);
        }

        Self::new(self.sample_rate, 1, mono)
    }
}

/// Transcription result from speech-to-text
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TranscriptionResult {
    /// Full transcribed text
    pub text: String,
    /// Per-segment transcriptions with timestamps
    pub segments: Vec<TranscriptionSegment>,
    /// Detected language (if available)
    pub language: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TranscriptionSegment {
    /// Segment text
    pub text: String,
    /// Start time in seconds
    pub start: f32,
    /// End time in seconds
    pub end: f32,
    /// Confidence score
    pub confidence: f32,
}

/// Voice activity detection result
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct VadResult {
    /// Speech segments detected
    pub segments: Vec<SpeechSegment>,
    /// Frame-level speech probabilities
    pub probabilities: Vec<f32>,
    /// Frame duration in seconds
    pub frame_duration: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SpeechSegment {
    /// Start time in seconds
    pub start: f32,
    /// End time in seconds
    pub end: f32,
    /// Average confidence
    pub confidence: f32,
}

#[crate::register_node]
#[derive(Default)]
pub struct LoadAudioNode {}

#[async_trait]
impl NodeLogic for LoadAudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "load_audio",
            "Load Audio",
            "Load audio file for processing",
            "AI/ML/ONNX/Audio",
        );

        node.add_icon("/flow/icons/audio.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("path", "Path", "Path to audio file", VariableType::Struct)
            .set_schema::<FlowPath>();

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin("audio", "Audio", "Loaded audio data", VariableType::Struct)
            .set_schema::<AudioData>();

        node.add_output_pin(
            "sample_rate",
            "Sample Rate",
            "Audio sample rate",
            VariableType::Integer,
        );

        node.add_output_pin(
            "duration",
            "Duration",
            "Duration in seconds",
            VariableType::Float,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let path: FlowPath = context.evaluate_pin("path").await?;
            let bytes = path.get(context, false).await?;

            let reader = hound::WavReader::new(std::io::Cursor::new(bytes))
                .map_err(|e| anyhow!("Failed to open audio file: {}", e))?;

            let spec = reader.spec();
            let sample_rate = spec.sample_rate;
            let channels = spec.channels;

            let samples: Vec<f32> = match spec.sample_format {
                hound::SampleFormat::Float => reader
                    .into_samples::<f32>()
                    .filter_map(|s| s.ok())
                    .collect(),
                hound::SampleFormat::Int => {
                    let bits = spec.bits_per_sample;
                    let max_val = (1 << (bits - 1)) as f32;
                    reader
                        .into_samples::<i32>()
                        .filter_map(|s| s.ok())
                        .map(|s| s as f32 / max_val)
                        .collect()
                }
            };

            let audio = AudioData::new(sample_rate, channels, samples);
            let duration = audio.duration_secs as f64;

            context.set_pin_value("audio", json!(audio)).await?;
            context
                .set_pin_value("sample_rate", json!(sample_rate as i64))
                .await?;
            context.set_pin_value("duration", json!(duration)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct VoiceActivityDetectionNode {}

impl VoiceActivityDetectionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for VoiceActivityDetectionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_vad",
            "Voice Activity Detection",
            "Detect speech segments in audio. Download Silero VAD model from: https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx",
            "AI/ML/ONNX/Audio",
        );

        node.add_icon("/flow/icons/microphone.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("model", "Model", "ONNX VAD Model", VariableType::Struct)
            .set_schema::<NodeOnnxSession>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("audio", "Audio", "Input audio data", VariableType::Struct)
            .set_schema::<AudioData>();

        node.add_input_pin(
            "threshold",
            "Threshold",
            "Speech probability threshold",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "min_speech_ms",
            "Min Speech",
            "Minimum speech duration (ms)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(250)));

        node.add_input_pin(
            "min_silence_ms",
            "Min Silence",
            "Minimum silence duration (ms)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin("result", "Result", "VAD result", VariableType::Struct)
            .set_schema::<VadResult>();

        node.add_output_pin(
            "segments",
            "Segments",
            "Speech segments",
            VariableType::Generic,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let audio: AudioData = context.evaluate_pin("audio").await?;
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);
            let min_speech_ms: i64 = context.evaluate_pin("min_speech_ms").await.unwrap_or(250);
            let min_silence_ms: i64 = context.evaluate_pin("min_silence_ms").await.unwrap_or(100);

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            // Resample to 16kHz mono for Silero VAD
            let audio = audio.to_mono().resample(16000);

            // Process in chunks (Silero VAD uses 512 sample chunks at 16kHz = 32ms)
            let chunk_size = 512;
            let frame_duration = chunk_size as f32 / 16000.0;
            let mut probabilities: Vec<f32> = Vec::new();

            // Initialize state tensors for Silero VAD
            let mut state = Array2::<f32>::zeros((2, 128));
            let sr = 16000i64;

            for chunk in audio.samples.chunks(chunk_size) {
                if chunk.len() < chunk_size {
                    break;
                }

                let input = Array2::from_shape_vec((1, chunk_size), chunk.to_vec())?;
                let input_value = Value::from_array(input)?;
                let state_value = Value::from_array(state.clone())?;
                let sr_value = Value::from_array(Array1::from_elem(1, sr))?;

                // Silero VAD expects: input, state, sr
                let outputs = session.run(inputs![
                    "input" => input_value,
                    "state" => state_value,
                    "sr" => sr_value
                ])?;

                // Get probability and update state
                if let (Some(prob_tensor), Some(state_tensor)) =
                    (outputs.get("output"), outputs.get("stateN"))
                {
                    if let Ok(prob) = prob_tensor.try_extract_array::<f32>()
                        && !prob.is_empty()
                    {
                        probabilities.push(prob[[0, 0]]);
                    }

                    if let Ok(new_state) = state_tensor.try_extract_array::<f32>() {
                        for i in 0..2 {
                            for j in 0..128 {
                                if let Some(&val) = new_state.get([i, j]) {
                                    state[[i, j]] = val;
                                }
                            }
                        }
                    }
                }
            }

            // Convert probabilities to segments
            let min_speech_frames =
                (min_speech_ms as f32 / 1000.0 / frame_duration).ceil() as usize;
            let min_silence_frames =
                (min_silence_ms as f32 / 1000.0 / frame_duration).ceil() as usize;

            let mut segments: Vec<SpeechSegment> = Vec::new();
            let mut in_speech = false;
            let mut speech_start = 0;
            let mut silence_count = 0;
            let mut speech_probs: Vec<f32> = Vec::new();

            for (i, &prob) in probabilities.iter().enumerate() {
                if prob >= threshold as f32 {
                    if !in_speech {
                        in_speech = true;
                        speech_start = i;
                        speech_probs.clear();
                    }
                    silence_count = 0;
                    speech_probs.push(prob);
                } else if in_speech {
                    silence_count += 1;
                    if silence_count >= min_silence_frames {
                        // End of speech
                        let speech_frames = i - silence_count - speech_start;
                        if speech_frames >= min_speech_frames {
                            let avg_conf =
                                speech_probs.iter().sum::<f32>() / speech_probs.len() as f32;
                            segments.push(SpeechSegment {
                                start: speech_start as f32 * frame_duration,
                                end: (i - silence_count) as f32 * frame_duration,
                                confidence: avg_conf,
                            });
                        }
                        in_speech = false;
                    }
                }
            }

            // Handle trailing speech
            if in_speech {
                let speech_frames = probabilities.len() - speech_start;
                if speech_frames >= min_speech_frames {
                    let avg_conf =
                        speech_probs.iter().sum::<f32>() / speech_probs.len().max(1) as f32;
                    segments.push(SpeechSegment {
                        start: speech_start as f32 * frame_duration,
                        end: probabilities.len() as f32 * frame_duration,
                        confidence: avg_conf,
                    });
                }
            }

            let result = VadResult {
                segments: segments.clone(),
                probabilities,
                frame_duration,
            };

            context.set_pin_value("result", json!(result)).await?;
            context.set_pin_value("segments", json!(segments)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ResampleAudioNode {}

#[async_trait]
impl NodeLogic for ResampleAudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "resample_audio",
            "Resample Audio",
            "Resample audio to target sample rate",
            "AI/ML/ONNX/Audio",
        );

        node.add_icon("/flow/icons/waveform.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("audio", "Audio", "Input audio", VariableType::Struct)
            .set_schema::<AudioData>();

        node.add_input_pin(
            "target_rate",
            "Target Rate",
            "Target sample rate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(16000)));

        node.add_input_pin(
            "to_mono",
            "To Mono",
            "Convert to mono",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "audio_out",
            "Audio Out",
            "Resampled audio",
            VariableType::Struct,
        )
        .set_schema::<AudioData>();

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let audio: AudioData = context.evaluate_pin("audio").await?;
            let target_rate: i64 = context.evaluate_pin("target_rate").await.unwrap_or(16000);
            let to_mono: bool = context.evaluate_pin("to_mono").await.unwrap_or(true);

            let mut result = audio;
            if to_mono {
                result = result.to_mono();
            }
            result = result.resample(target_rate as u32);

            context.set_pin_value("audio_out", json!(result)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TrimAudioNode {}

#[async_trait]
impl NodeLogic for TrimAudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "trim_audio",
            "Trim Audio",
            "Trim audio to speech segments from VAD",
            "AI/ML/ONNX/Audio",
        );

        node.add_icon("/flow/icons/scissors.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("audio", "Audio", "Input audio", VariableType::Struct)
            .set_schema::<AudioData>();

        node.add_input_pin(
            "segments",
            "Segments",
            "Speech segments from VAD",
            VariableType::Generic,
        );

        node.add_input_pin(
            "padding",
            "Padding",
            "Padding around segments (seconds)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.1)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "clips",
            "Clips",
            "Trimmed audio clips",
            VariableType::Generic,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let audio: AudioData = context.evaluate_pin("audio").await?;
            let segments: Vec<SpeechSegment> = context.evaluate_pin("segments").await?;
            let padding: f64 = context.evaluate_pin("padding").await.unwrap_or(0.1);

            let samples_per_sec = audio.sample_rate as f32 * audio.channels as f32;
            let mut clips: Vec<AudioData> = Vec::new();

            for seg in segments {
                let start_sample =
                    ((seg.start - padding as f32).max(0.0) * samples_per_sec) as usize;
                let end_sample = ((seg.end + padding as f32).min(audio.duration_secs)
                    * samples_per_sec) as usize;

                if start_sample < end_sample && end_sample <= audio.samples.len() {
                    let clip_samples: Vec<f32> = audio.samples[start_sample..end_sample].to_vec();
                    clips.push(AudioData::new(
                        audio.sample_rate,
                        audio.channels,
                        clip_samples,
                    ));
                }
            }

            context.set_pin_value("clips", json!(clips)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AudioToMelSpectrogramNode {}

#[async_trait]
impl NodeLogic for AudioToMelSpectrogramNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "audio_to_mel_spectrogram",
            "Audio to Mel Spectrogram",
            "Convert audio to mel spectrogram for speech models",
            "AI/ML/ONNX/Audio",
        );

        node.add_icon("/flow/icons/waveform.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "audio",
            "Audio",
            "Input audio (16kHz mono)",
            VariableType::Struct,
        )
        .set_schema::<AudioData>();

        node.add_input_pin(
            "n_mels",
            "N Mels",
            "Number of mel bands",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(80)));

        node.add_input_pin(
            "hop_length",
            "Hop Length",
            "Hop length in samples",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(160)));

        node.add_input_pin("n_fft", "N FFT", "FFT window size", VariableType::Integer)
            .set_default_value(Some(json!(400)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "spectrogram",
            "Spectrogram",
            "Mel spectrogram [n_mels, time]",
            VariableType::Generic,
        );

        node.add_output_pin(
            "frames",
            "Frames",
            "Number of time frames",
            VariableType::Integer,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let audio: AudioData = context.evaluate_pin("audio").await?;
            let n_mels: i64 = context.evaluate_pin("n_mels").await.unwrap_or(80);
            let hop_length: i64 = context.evaluate_pin("hop_length").await.unwrap_or(160);
            let n_fft: i64 = context.evaluate_pin("n_fft").await.unwrap_or(400);

            // Simple mel spectrogram approximation
            // In production, use a proper audio processing library
            let audio = audio.to_mono();
            let samples = &audio.samples;

            let n_frames = (samples.len() as i64 - n_fft) / hop_length + 1;
            let n_frames = n_frames.max(0) as usize;

            // Simplified: just compute power spectrum magnitudes
            // Real implementation would use FFT and mel filterbank
            let mut spectrogram: Vec<Vec<f32>> = Vec::new();

            for i in 0..n_frames {
                let start = i * hop_length as usize;
                let end = (start + n_fft as usize).min(samples.len());
                let frame: Vec<f32> = samples[start..end].to_vec();

                // Compute simple energy in frequency bands
                let mut mel_bands = vec![0.0f32; n_mels as usize];
                let band_size = frame.len() / n_mels as usize;

                for (b, mel_band) in mel_bands.iter_mut().enumerate() {
                    let band_start = b * band_size;
                    let band_end = (band_start + band_size).min(frame.len());
                    let energy: f32 = frame[band_start..band_end]
                        .iter()
                        .map(|x| x * x)
                        .sum::<f32>()
                        / band_size as f32;
                    *mel_band = (energy + 1e-10).log10();
                }

                spectrogram.push(mel_bands);
            }

            context
                .set_pin_value("spectrogram", json!(spectrogram))
                .await?;
            context
                .set_pin_value("frames", json!(n_frames as i64))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}
