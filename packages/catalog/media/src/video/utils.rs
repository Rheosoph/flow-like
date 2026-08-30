#[cfg(feature = "execute")]
use bytes::Bytes;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_storage::{Path as ObjectPath, object_store::ObjectStore};
use flow_like_types::{
    async_trait,
    json::{Deserialize, Serialize, json},
};
use schemars::JsonSchema;
#[cfg(feature = "execute")]
use std::{collections::BTreeSet, sync::Arc};
#[cfg(feature = "execute")]
use video_utils_rs::{Decoder, Encoder, VideoDecoder, VideoEncoder};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoStreamInfo {
    pub track_id: u32,
    pub media_type: String,
    pub codec: String,
    pub time_base_num: i32,
    pub time_base_den: i32,
    pub duration_seconds: Option<f64>,
    pub fps: Option<f64>,
    pub frame_count: Option<u64>,
    pub packet_count: Option<u64>,
    pub average_frame_duration_seconds: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub language: Option<String>,
    pub codec_config_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoMediaInfo {
    pub duration_seconds: Option<f64>,
    pub streams: Vec<VideoStreamInfo>,
    pub tags: Vec<MediaTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaTag {
    pub key: String,
    pub value: String,
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone, Copy, Default)]
struct StreamTimingStats {
    fps: Option<f64>,
    frame_count: Option<u64>,
    packet_count: Option<u64>,
    average_frame_duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemuxStreamDecision {
    pub track_id: u32,
    pub media_type: String,
    pub codec: String,
    pub action: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemuxCompatibilityReport {
    pub compatible: bool,
    pub packet_copy_only: bool,
    pub requires_transcode: bool,
    pub has_unsupported_streams: bool,
    pub source_format: Option<String>,
    pub target_format: Option<String>,
    pub streams: Vec<RemuxStreamDecision>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleCue {
    pub index: Option<usize>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodecBackendInfo {
    pub kind: String,
    pub target: String,
    pub source: String,
    pub probe: String,
    pub hardware_accelerated: bool,
    pub decodes: Vec<String>,
    pub encodes: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoUtilsFeatureSet {
    pub packet_ops: bool,
    pub audio_core: bool,
    pub audio_io: bool,
    pub frame_core: bool,
    pub image_io: bool,
    pub preview: bool,
    pub subtitles: bool,
    pub streaming: bool,
    pub platform_codecs: bool,
    pub codec_apple: bool,
    pub codec_android: bool,
    pub codec_windows: bool,
    pub codec_gstreamer: bool,
    pub codec_web: bool,
    pub codec_h264_rust: bool,
    pub codec_h265_rust: bool,
    pub codec_av1_rust: bool,
    pub codec_openh264_ffi: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodecSupportInfo {
    pub codec: String,
    pub media_type: Option<String>,
    pub implementation: String,
    pub can_decode: bool,
    pub can_encode: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlatformCodecProbeInfo {
    pub codec: String,
    pub direction: String,
    pub supported: bool,
    pub backend: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackendSelectionInfo {
    pub codec: String,
    pub direction: String,
    pub backend: Option<String>,
    pub found: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoTransformReport {
    pub source: String,
    pub target: String,
    pub source_format: String,
    pub target_format: String,
    pub video_track_id: u32,
    pub input_packets: usize,
    pub decoded_frames: usize,
    pub encoded_video_packets: usize,
    pub copied_packets: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoTranscodeReport {
    pub source: String,
    pub target: String,
    pub source_format: String,
    pub target_format: String,
    pub operation: String,
    pub video_track_id: Option<u32>,
    pub input_packets: usize,
    pub output_packets: usize,
    pub decoded_video_frames: usize,
    pub encoded_video_packets: usize,
    pub copied_packets: usize,
    pub dropped_packets: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleBurnInReport {
    pub source: String,
    pub sidecar: String,
    pub target: String,
    pub source_format: String,
    pub target_format: String,
    pub video_track_id: u32,
    pub event_count: usize,
    pub decoded_frames: usize,
    pub encoded_video_packets: usize,
    pub copied_packets: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AudioTransformReport {
    pub source: String,
    pub target: String,
    pub source_format: String,
    pub target_format: String,
    pub audio_track_id: u32,
    pub input_packets: usize,
    pub decoded_frames: usize,
    pub encoded_audio_packets: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImageOperationReport {
    pub source: String,
    pub target: String,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub output_format: String,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoRemuxReport {
    pub source: String,
    pub target: String,
    pub operation: String,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleTrackMuxReport {
    pub source: String,
    pub sidecar: String,
    pub target: String,
    pub event_count: usize,
    pub subtitle_packets: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleTrackExtractReport {
    pub source: String,
    pub target: String,
    pub subtitle_track_id: u32,
    pub event_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VideoFrameExtractionReport {
    pub source: String,
    pub target: String,
    pub video_track_id: u32,
    pub frame_index: usize,
    pub decoded_frames: usize,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub output_format: String,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContactSheetReport {
    pub source: String,
    pub target: String,
    pub video_track_id: u32,
    pub frame_count: usize,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub output_format: String,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HlsVodPackageReport {
    pub init_segment: Option<FlowPath>,
    pub segment_count: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContainerDetectionInfo {
    pub format: String,
    pub display_name: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WaveformBucketInfo {
    pub start_sample: usize,
    pub end_sample: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SilenceRangeInfo {
    pub start_sample: usize,
    pub end_sample: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AudioAnalysisReport {
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub decoded_frames: usize,
    pub sample_frames: usize,
    pub peak_amplitude: f32,
    pub rms: f32,
    pub waveform_buckets: usize,
    pub silence_ranges: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BitstreamConvertReport {
    pub source: String,
    pub target: String,
    pub codec: String,
    pub conversion: String,
    pub track_id: u32,
    pub packets: usize,
    pub bytes_written: u64,
}

fn add_exec_pins(node: &mut Node) {
    node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
    node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
}

fn add_flow_path_input(node: &mut Node, id: &str, name: &str, description: &str) {
    node.add_input_pin(id, name, description, VariableType::Struct)
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_flow_path_output(node: &mut Node, id: &str, name: &str, description: &str) {
    node.add_output_pin(id, name, description, VariableType::Struct)
        .set_schema::<FlowPath>();
}

fn add_video_icon_and_scores(node: &mut Node) {
    node.add_icon("/flow/icons/video.svg");
    node.set_scores(
        NodeScores::new()
            .set_privacy(8)
            .set_security(7)
            .set_performance(7)
            .set_governance(8)
            .set_reliability(7)
            .set_cost(10)
            .build(),
    );
}

#[cfg(not(feature = "execute"))]
fn execute_feature_error() -> flow_like_types::Error {
    flow_like_types::anyhow!("Requires the 'execute' feature")
}

#[cfg(feature = "execute")]
async fn flow_path_object(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
) -> flow_like_types::Result<(Arc<dyn ObjectStore>, ObjectPath)> {
    let runtime = flow_path.to_runtime(context).await?;
    Ok((runtime.store.as_generic(), runtime.path))
}

#[cfg(feature = "execute")]
fn flow_path_from_object_path(path: &ObjectPath, template: &FlowPath) -> FlowPath {
    FlowPath::new(
        path.to_string(),
        template.store_ref.clone(),
        template.cache_store_ref.clone(),
    )
}

#[cfg(feature = "execute")]
fn optional_track_id(value: i64) -> flow_like_types::Result<Option<u32>> {
    if value <= 0 {
        return Ok(None);
    }
    Ok(Some(u32::try_from(value)?))
}

#[cfg(feature = "execute")]
fn subtitle_format(value: &str) -> flow_like_types::Result<video_utils_rs::SubtitleFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "srt" => Ok(video_utils_rs::SubtitleFormat::Srt),
        "webvtt" | "vtt" => Ok(video_utils_rs::SubtitleFormat::WebVtt),
        other => Err(flow_like_types::anyhow!(
            "Unsupported subtitle format: {}",
            other
        )),
    }
}

#[cfg(feature = "execute")]
fn subtitle_text(
    format: video_utils_rs::SubtitleFormat,
    events: &[video_utils_rs::SubtitleEvent],
) -> String {
    match format {
        video_utils_rs::SubtitleFormat::Srt => video_utils_rs::write_srt(events),
        video_utils_rs::SubtitleFormat::WebVtt => video_utils_rs::write_webvtt(events),
    }
}

#[cfg(feature = "execute")]
fn clean_optional(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(feature = "execute")]
fn media_type_name(media_type: video_utils_rs::MediaType) -> String {
    format!("{media_type:?}").to_ascii_lowercase()
}

#[cfg(feature = "execute")]
fn stream_to_info(stream: &video_utils_rs::StreamInfo) -> VideoStreamInfo {
    stream_to_info_with_stats(stream, StreamTimingStats::default())
}

#[cfg(feature = "execute")]
fn stream_to_info_with_stats(
    stream: &video_utils_rs::StreamInfo,
    stats: StreamTimingStats,
) -> VideoStreamInfo {
    VideoStreamInfo {
        track_id: stream.track_id,
        media_type: media_type_name(stream.media_type),
        codec: stream.codec.to_string(),
        time_base_num: stream.time_base.num,
        time_base_den: stream.time_base.den,
        duration_seconds: stream.duration_seconds(),
        fps: stats.fps,
        frame_count: stats.frame_count,
        packet_count: stats.packet_count,
        average_frame_duration_seconds: stats.average_frame_duration_seconds,
        width: stream.width,
        height: stream.height,
        sample_rate: stream.sample_rate,
        channels: stream.channels,
        language: stream.language.clone(),
        codec_config_bytes: stream.codec_config.as_ref().map(|config| config.len()),
    }
}

#[cfg(feature = "execute")]
fn stream_timing_stats(
    stream: &video_utils_rs::StreamInfo,
    packets: &[video_utils_rs::EncodedPacket],
) -> StreamTimingStats {
    let mut packet_count = 0_u64;
    let mut first_packet_duration_seconds = None;
    let mut first_pts_seconds = None;
    let mut last_end_seconds = None;

    for packet in packets
        .iter()
        .filter(|packet| packet.track_id == stream.track_id)
    {
        packet_count += 1;

        let start_seconds = packet.time_base.ticks_to_seconds(packet.pts);
        let end_seconds = packet.time_base.ticks_to_seconds(packet.end_pts());

        first_pts_seconds = Some(
            first_pts_seconds
                .map(|current: f64| current.min(start_seconds))
                .unwrap_or(start_seconds),
        );
        last_end_seconds = Some(
            last_end_seconds
                .map(|current: f64| current.max(end_seconds))
                .unwrap_or(end_seconds),
        );

        if packet.duration > 0 && first_packet_duration_seconds.is_none() {
            let duration_seconds = packet.duration_seconds();
            if duration_seconds.is_finite() && duration_seconds > 0.0 {
                first_packet_duration_seconds = Some(duration_seconds);
            }
        }
    }

    let packet_count = Some(packet_count);
    if stream.media_type != video_utils_rs::MediaType::Video {
        return StreamTimingStats {
            packet_count,
            ..Default::default()
        };
    }

    let frame_count = packet_count.filter(|count| *count > 0);
    let span_seconds = first_pts_seconds
        .zip(last_end_seconds)
        .map(|(first, last)| last - first)
        .filter(|duration| duration.is_finite() && *duration > 0.0);
    let duration_seconds = stream
        .duration_seconds()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .or(span_seconds);

    let fps = frame_count.and_then(|count| {
        duration_seconds
            .map(|duration| count as f64 / duration)
            .or_else(|| first_packet_duration_seconds.map(|duration| 1.0 / duration))
            .filter(|fps| fps.is_finite() && *fps > 0.0)
    });
    let average_frame_duration_seconds = fps.map(|fps| 1.0 / fps);

    StreamTimingStats {
        fps,
        frame_count,
        packet_count,
        average_frame_duration_seconds,
    }
}

#[cfg(feature = "execute")]
fn media_to_info(media: &video_utils_rs::MediaInfo) -> VideoMediaInfo {
    VideoMediaInfo {
        duration_seconds: media.duration_seconds,
        streams: media.streams.iter().map(stream_to_info).collect(),
        tags: media
            .tags
            .iter()
            .map(|(key, value)| MediaTag {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

#[cfg(feature = "execute")]
fn media_to_info_with_packets(
    media: &video_utils_rs::MediaInfo,
    packets: &[video_utils_rs::EncodedPacket],
) -> VideoMediaInfo {
    VideoMediaInfo {
        duration_seconds: media.duration_seconds,
        streams: media
            .streams
            .iter()
            .map(|stream| stream_to_info_with_stats(stream, stream_timing_stats(stream, packets)))
            .collect(),
        tags: media
            .tags
            .iter()
            .map(|(key, value)| MediaTag {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

#[cfg(feature = "execute")]
fn remux_plan_report(plan: &video_utils_rs::RemuxPlan) -> RemuxCompatibilityReport {
    let streams = plan
        .streams
        .iter()
        .map(|stream| {
            let (action, reason) = match &stream.action {
                video_utils_rs::RemuxAction::PacketCopy => ("packet_copy", None),
                video_utils_rs::RemuxAction::TranscodeRequired { reason } => {
                    ("transcode_required", Some(reason.clone()))
                }
                video_utils_rs::RemuxAction::Unsupported { reason } => {
                    ("unsupported", Some(reason.clone()))
                }
            };
            RemuxStreamDecision {
                track_id: stream.track_id,
                media_type: media_type_name(stream.media_type),
                codec: stream.codec.to_string(),
                action: action.to_owned(),
                reason,
            }
        })
        .collect();

    RemuxCompatibilityReport {
        compatible: plan.is_packet_copy_only(),
        packet_copy_only: plan.is_packet_copy_only(),
        requires_transcode: plan.requires_transcode(),
        has_unsupported_streams: plan.has_unsupported_streams(),
        source_format: Some(plan.source.as_str().to_owned()),
        target_format: Some(plan.target.as_str().to_owned()),
        streams,
        reason: None,
    }
}

#[cfg(feature = "execute")]
fn media_for_packet_subset(
    source: &video_utils_rs::MediaInfo,
    packets: &[video_utils_rs::EncodedPacket],
) -> video_utils_rs::MediaInfo {
    let track_ids = packets
        .iter()
        .map(|packet| packet.track_id)
        .collect::<BTreeSet<_>>();
    let mut media = video_utils_rs::MediaInfo {
        duration_seconds: packets
            .iter()
            .map(|packet| packet.time_base.ticks_to_seconds(packet.end_pts()))
            .max_by(f64::total_cmp),
        tags: source.tags.clone(),
        ..Default::default()
    };
    for stream in &source.streams {
        if track_ids.contains(&stream.track_id) {
            media.push_stream(stream.clone());
        }
    }
    media
}

#[cfg(feature = "execute")]
fn select_video_track_id(
    media: &video_utils_rs::MediaInfo,
    requested: Option<u32>,
) -> flow_like_types::Result<u32> {
    if let Some(track_id) = requested {
        let stream = media
            .stream(track_id)
            .ok_or_else(|| flow_like_types::anyhow!("Requested track {} is missing", track_id))?;
        if stream.media_type != video_utils_rs::MediaType::Video {
            return Err(flow_like_types::anyhow!(
                "Requested track {} is not a video track",
                track_id
            ));
        }
        return Ok(track_id);
    }

    media
        .video_streams()
        .next()
        .map(|stream| stream.track_id)
        .ok_or_else(|| flow_like_types::anyhow!("Source media has no video track"))
}

#[cfg(feature = "execute")]
fn cues_from_events(events: &[video_utils_rs::SubtitleEvent]) -> Vec<SubtitleCue> {
    events
        .iter()
        .map(|event| SubtitleCue {
            index: event.index,
            start_ms: event.start_ms,
            end_ms: event.end_ms,
            text: event.text.clone(),
        })
        .collect()
}

#[cfg(feature = "execute")]
fn events_from_cues(
    cues: Vec<SubtitleCue>,
) -> flow_like_types::Result<Vec<video_utils_rs::SubtitleEvent>> {
    cues.into_iter()
        .map(|cue| {
            let mut event = video_utils_rs::SubtitleEvent::new(cue.start_ms, cue.end_ms, cue.text)?;
            event.index = cue.index;
            Ok(event)
        })
        .collect()
}

#[cfg(feature = "execute")]
fn backend_info() -> Vec<CodecBackendInfo> {
    video_utils_rs::recommended_backends_for_current_target()
        .into_iter()
        .map(|backend| CodecBackendInfo {
            kind: format!("{:?}", backend.kind),
            target: format!("{:?}", backend.target),
            source: format!("{:?}", backend.source),
            probe: format!("{:?}", backend.probe),
            hardware_accelerated: backend.hardware_accelerated,
            decodes: backend.decodes.iter().map(ToString::to_string).collect(),
            encodes: backend.encodes.iter().map(ToString::to_string).collect(),
            note: backend.note.to_owned(),
        })
        .collect()
}

#[cfg(feature = "execute")]
fn feature_set() -> VideoUtilsFeatureSet {
    let features = video_utils_rs::compiled_features();
    VideoUtilsFeatureSet {
        packet_ops: features.packet_ops,
        audio_core: features.audio_core,
        audio_io: features.audio_io,
        frame_core: features.frame_core,
        image_io: features.image_io,
        preview: features.preview,
        subtitles: features.subtitles,
        streaming: features.streaming,
        platform_codecs: features.platform_codecs,
        codec_apple: features.codec_apple,
        codec_android: features.codec_android,
        codec_windows: features.codec_windows,
        codec_gstreamer: features.codec_gstreamer,
        codec_web: features.codec_web,
        codec_h264_rust: features.codec_h264_rust,
        codec_h265_rust: features.codec_h265_rust,
        codec_av1_rust: features.codec_av1_rust,
        codec_openh264_ffi: features.codec_openh264_ffi,
    }
}

#[cfg(feature = "execute")]
fn codec_id(value: &str) -> video_utils_rs::CodecId {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "h264" | "avc" | "avc1" => video_utils_rs::CodecId::H264,
        "h265" | "hevc" | "hvc1" | "hev1" => video_utils_rs::CodecId::H265,
        "av1" | "av01" => video_utils_rs::CodecId::AV1,
        "vp8" => video_utils_rs::CodecId::VP8,
        "vp9" => video_utils_rs::CodecId::VP9,
        "mpeg1" | "mpeg1video" | "mpeg-1-video" => video_utils_rs::CodecId::Mpeg1Video,
        "mpeg2" | "mpeg2video" | "mpeg-2-video" => video_utils_rs::CodecId::Mpeg2Video,
        "mpeg4" | "mpeg4-part2" | "mpeg-4-part-2" => video_utils_rs::CodecId::Mpeg4Part2,
        "prores" => video_utils_rs::CodecId::ProRes,
        "theora" => video_utils_rs::CodecId::Theora,
        "dirac" => video_utils_rs::CodecId::Dirac,
        "rawvideo" | "raw-video" => video_utils_rs::CodecId::RawVideo,
        "aac" => video_utils_rs::CodecId::Aac,
        "ac3" => video_utils_rs::CodecId::Ac3,
        "eac3" => video_utils_rs::CodecId::Eac3,
        "adpcm" => video_utils_rs::CodecId::Adpcm,
        "alac" => video_utils_rs::CodecId::Alac,
        "opus" => video_utils_rs::CodecId::Opus,
        "flac" => video_utils_rs::CodecId::Flac,
        "mp1" => video_utils_rs::CodecId::Mp1,
        "mp2" => video_utils_rs::CodecId::Mp2,
        "mp3" => video_utils_rs::CodecId::Mp3,
        "pcm" => video_utils_rs::CodecId::Pcm,
        "vorbis" => video_utils_rs::CodecId::Vorbis,
        "speex" => video_utils_rs::CodecId::Speex,
        "dts" => video_utils_rs::CodecId::Dts,
        "wma" => video_utils_rs::CodecId::Wma,
        "wavpack" => video_utils_rs::CodecId::WavPack,
        "png" => video_utils_rs::CodecId::Png,
        "jpg" | "jpeg" => video_utils_rs::CodecId::Jpeg,
        "gif" => video_utils_rs::CodecId::Gif,
        "webp" => video_utils_rs::CodecId::WebP,
        "avif" => video_utils_rs::CodecId::Avif,
        "srt" => video_utils_rs::CodecId::Srt,
        "webvtt" | "vtt" => video_utils_rs::CodecId::WebVtt,
        other => video_utils_rs::CodecId::Unknown(other.to_owned()),
    }
}

#[cfg(feature = "execute")]
fn codec_direction(value: &str) -> flow_like_types::Result<video_utils_rs::CodecDirection> {
    match value.trim().to_ascii_lowercase().as_str() {
        "decode" | "decoder" | "read" => Ok(video_utils_rs::CodecDirection::Decode),
        "encode" | "encoder" | "write" => Ok(video_utils_rs::CodecDirection::Encode),
        other => Err(flow_like_types::anyhow!(
            "Unsupported codec direction: {}",
            other
        )),
    }
}

#[cfg(feature = "execute")]
fn platform_probe_info(probe: video_utils_rs::PlatformCodecProbe) -> PlatformCodecProbeInfo {
    PlatformCodecProbeInfo {
        codec: probe.codec.to_string(),
        direction: format!("{:?}", probe.direction).to_ascii_lowercase(),
        supported: probe.supported,
        backend: probe.backend.map(|backend| format!("{backend:?}")),
        detail: probe.detail,
    }
}

#[cfg(feature = "execute")]
fn codec_support_info() -> Vec<CodecSupportInfo> {
    video_utils_rs::CodecRegistry::builtin()
        .support()
        .iter()
        .map(|support| CodecSupportInfo {
            codec: support.codec.to_string(),
            media_type: support.media_type.map(media_type_name),
            implementation: format!("{:?}", support.kind),
            can_decode: support.can_decode,
            can_encode: support.can_encode,
            note: support.note.to_owned(),
        })
        .collect()
}

#[cfg(feature = "execute")]
fn image_format(value: &str) -> flow_like_types::Result<video_utils_rs::ImageStillFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "png" => Ok(video_utils_rs::ImageStillFormat::Png),
        "jpg" | "jpeg" => Ok(video_utils_rs::ImageStillFormat::Jpeg),
        "gif" => Ok(video_utils_rs::ImageStillFormat::Gif),
        "webp" => Ok(video_utils_rs::ImageStillFormat::WebP),
        "avif" => Ok(video_utils_rs::ImageStillFormat::Avif),
        other => Err(flow_like_types::anyhow!(
            "Unsupported image format: {}",
            other
        )),
    }
}

#[cfg(feature = "execute")]
fn image_format_for_target(
    requested: &str,
    target: &ObjectPath,
) -> flow_like_types::Result<video_utils_rs::ImageStillFormat> {
    let requested = requested.trim();
    if !requested.is_empty() && !requested.eq_ignore_ascii_case("auto") {
        return image_format(requested);
    }
    let extension = target.extension().ok_or_else(|| {
        flow_like_types::anyhow!("Target path needs an image extension when format is auto")
    })?;
    image_format(extension)
}

#[cfg(feature = "execute")]
fn image_encoder(format: video_utils_rs::ImageStillFormat) -> video_utils_rs::ImageRgbaEncoder {
    video_utils_rs::ImageRgbaEncoder::with_format(format)
}

#[cfg(feature = "execute")]
fn image_format_name(format: video_utils_rs::ImageStillFormat) -> &'static str {
    match format {
        video_utils_rs::ImageStillFormat::Png => "png",
        video_utils_rs::ImageStillFormat::Jpeg => "jpeg",
        video_utils_rs::ImageStillFormat::Gif => "gif",
        video_utils_rs::ImageStillFormat::WebP => "webp",
        video_utils_rs::ImageStillFormat::Avif => "avif",
    }
}

#[cfg(feature = "execute")]
fn fade_shape(value: &str) -> flow_like_types::Result<video_utils_rs::FadeShape> {
    match value.trim().to_ascii_lowercase().as_str() {
        "linear" => Ok(video_utils_rs::FadeShape::Linear),
        "equal_power" | "equal-power" | "power" => Ok(video_utils_rs::FadeShape::EqualPower),
        other => Err(flow_like_types::anyhow!(
            "Unsupported fade shape: {}",
            other
        )),
    }
}

#[cfg(feature = "execute")]
fn audio_pipeline(
    gain_factor: f64,
    gain_db: f64,
    normalize_peak: f64,
    fade_in_seconds: f64,
    fade_out_seconds: f64,
    fade_shape_name: &str,
    sample_rate: Option<u32>,
) -> flow_like_types::Result<video_utils_rs::AudioTransformPipeline> {
    let mut pipeline = video_utils_rs::AudioTransformPipeline::new();
    if (gain_factor - 1.0).abs() > f64::EPSILON {
        pipeline.push(video_utils_rs::AudioTransform::Gain {
            factor: gain_factor as f32,
        });
    }
    if gain_db.abs() > f64::EPSILON {
        pipeline.push(video_utils_rs::AudioTransform::GainDb { db: gain_db as f32 });
    }
    if normalize_peak > 0.0 {
        pipeline.push(video_utils_rs::AudioTransform::NormalizePeak {
            target_peak: normalize_peak as f32,
        });
    }
    if fade_in_seconds > 0.0 || fade_out_seconds > 0.0 {
        let sample_rate = sample_rate.unwrap_or(48_000);
        pipeline.push(video_utils_rs::AudioTransform::Fade {
            fade_in_samples: (fade_in_seconds * sample_rate as f64).round() as usize,
            fade_out_samples: (fade_out_seconds * sample_rate as f64).round() as usize,
            shape: fade_shape(fade_shape_name)?,
        });
    }
    Ok(pipeline)
}

#[cfg(feature = "execute")]
fn audio_transform_report(
    report: video_utils_rs::ObjectAudioTransformReport,
) -> AudioTransformReport {
    AudioTransformReport {
        source: report.source.to_string(),
        target: report.target.to_string(),
        source_format: report.source_format.as_str().to_owned(),
        target_format: report.target_format.as_str().to_owned(),
        audio_track_id: report.audio_track_id,
        input_packets: report.input_packets,
        decoded_frames: report.decoded_frames,
        encoded_audio_packets: report.encoded_audio_packets,
        bytes_written: report.bytes_written,
    }
}

#[cfg(feature = "execute")]
fn concat_audio_frames(
    frames: &[video_utils_rs::AudioFrame],
) -> flow_like_types::Result<video_utils_rs::AudioFrame> {
    let first = frames
        .first()
        .ok_or_else(|| flow_like_types::anyhow!("Audio decoder returned no frames"))?;
    let sample_rate = first.sample_rate;
    let channels = first.channels;
    let mut samples = Vec::new();
    for frame in frames {
        if frame.sample_rate != sample_rate || frame.channels != channels {
            return Err(flow_like_types::anyhow!(
                "Decoded audio changes format mid-stream"
            ));
        }
        samples.extend_from_slice(&frame.samples_f32_interleaved);
    }
    Ok(video_utils_rs::AudioFrame::new(
        sample_rate,
        channels,
        0,
        samples,
    )?)
}

#[cfg(feature = "execute")]
fn waveform_bucket_info(
    bucket: video_utils_rs::WaveformBucket,
    sample_rate: u32,
) -> WaveformBucketInfo {
    WaveformBucketInfo {
        start_sample: bucket.start_sample,
        end_sample: bucket.end_sample,
        start_seconds: bucket.start_sample as f64 / sample_rate as f64,
        end_seconds: bucket.end_sample as f64 / sample_rate as f64,
        min: bucket.min,
        max: bucket.max,
        rms: bucket.rms,
    }
}

#[cfg(feature = "execute")]
fn silence_range_info(range: video_utils_rs::SilenceRange, sample_rate: u32) -> SilenceRangeInfo {
    let start_seconds = range.start_sample as f64 / sample_rate as f64;
    let end_seconds = range.end_sample as f64 / sample_rate as f64;
    SilenceRangeInfo {
        start_sample: range.start_sample,
        end_sample: range.end_sample,
        start_seconds,
        end_seconds,
        duration_seconds: end_seconds - start_seconds,
    }
}

#[cfg(feature = "execute")]
async fn decode_audio_file(
    store: &dyn ObjectStore,
    path: &ObjectPath,
) -> flow_like_types::Result<Vec<video_utils_rs::AudioFrame>> {
    let bytes = video_utils_rs::read_object_bytes(store, path).await?;
    let mut decoder = path
        .extension()
        .map(video_utils_rs::SymphoniaAudioDecoder::with_extension)
        .unwrap_or_default();
    Ok(decoder.decode(&bytes)?)
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
fn video_frame_pipeline(
    crop_x: i64,
    crop_y: i64,
    crop_width: i64,
    crop_height: i64,
    resize_width: i64,
    resize_height: i64,
    flip_horizontal: bool,
    flip_vertical: bool,
    rotate_degrees: i64,
    blur_radius: i64,
    brightness: f64,
    contrast: f64,
    saturation: f64,
) -> flow_like_types::Result<video_utils_rs::FrameTransformPipeline> {
    let mut pipeline = video_utils_rs::FrameTransformPipeline::new();
    if crop_width > 0 && crop_height > 0 {
        pipeline.push(video_utils_rs::FrameTransform::Crop(
            video_utils_rs::CropRect::new(
                u32::try_from(crop_x.max(0))?,
                u32::try_from(crop_y.max(0))?,
                u32::try_from(crop_width)?,
                u32::try_from(crop_height)?,
            ),
        ));
    }
    if resize_width > 0 && resize_height > 0 {
        pipeline.push(video_utils_rs::FrameTransform::Resize {
            width: u32::try_from(resize_width)?,
            height: u32::try_from(resize_height)?,
        });
    }
    if flip_horizontal {
        pipeline.push(video_utils_rs::FrameTransform::FlipHorizontal);
    }
    if flip_vertical {
        pipeline.push(video_utils_rs::FrameTransform::FlipVertical);
    }
    let normalized_rotation = rotate_degrees.rem_euclid(360);
    match normalized_rotation {
        0 => {}
        90 => pipeline.push(video_utils_rs::FrameTransform::Rotate90 { clockwise: true }),
        270 => pipeline.push(video_utils_rs::FrameTransform::Rotate90 { clockwise: false }),
        180 => {
            pipeline.push(video_utils_rs::FrameTransform::Rotate90 { clockwise: true });
            pipeline.push(video_utils_rs::FrameTransform::Rotate90 { clockwise: true });
        }
        _ => {
            return Err(flow_like_types::anyhow!(
                "rotate_degrees must be one of 0, 90, 180, or 270"
            ));
        }
    }
    if blur_radius > 0 {
        pipeline.push(video_utils_rs::FrameTransform::BoxBlur {
            radius: u32::try_from(blur_radius)?,
        });
    }
    if brightness.abs() > f64::EPSILON
        || (contrast - 1.0).abs() > f64::EPSILON
        || (saturation - 1.0).abs() > f64::EPSILON
    {
        pipeline.push(video_utils_rs::FrameTransform::ColorFilter(
            video_utils_rs::ColorFilter {
                brightness: brightness as f32,
                contrast: contrast as f32,
                saturation: saturation as f32,
                alpha: 1.0,
            },
        ));
    }
    Ok(pipeline)
}

#[cfg(feature = "execute")]
fn selected_video_stream(
    media: &video_utils_rs::MediaInfo,
    track_id: Option<u32>,
) -> flow_like_types::Result<&video_utils_rs::StreamInfo> {
    let selected = select_video_track_id(media, track_id)?;
    media
        .stream(selected)
        .ok_or_else(|| flow_like_types::anyhow!("Selected video track {} is missing", selected))
}

#[cfg(feature = "execute")]
fn packet_frame_duration(packets: &[video_utils_rs::EncodedPacket], track_id: u32) -> i64 {
    packets
        .iter()
        .find(|packet| packet.track_id == track_id && packet.duration > 0)
        .map(|packet| packet.duration)
        .or_else(|| {
            let mut pts = packets
                .iter()
                .filter(|packet| packet.track_id == track_id)
                .map(|packet| packet.pts)
                .collect::<Vec<_>>();
            pts.sort_unstable();
            pts.windows(2)
                .find_map(|window| (window[1] > window[0]).then_some(window[1] - window[0]))
        })
        .unwrap_or(1)
}

#[cfg(feature = "execute")]
fn platform_video_decoder(
    stream: &video_utils_rs::StreamInfo,
) -> flow_like_types::Result<video_utils_rs::PlatformVideoDecoder> {
    let width = stream
        .width
        .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no width"))?;
    let height = stream
        .height
        .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no height"))?;
    let mut config =
        video_utils_rs::PlatformVideoDecoderConfig::new(stream.codec.clone(), width, height);
    if let Some(codec_config) = &stream.codec_config {
        config = config.with_extra_data(codec_config.to_vec());
    }
    Ok(video_utils_rs::PlatformVideoDecoder::new(config)?)
}

#[cfg(feature = "execute")]
fn platform_video_encoder(
    codec: video_utils_rs::CodecId,
    width: u32,
    height: u32,
    time_base: video_utils_rs::TimeBase,
    frame_duration: i64,
    bitrate: i64,
) -> flow_like_types::Result<video_utils_rs::PlatformVideoEncoder> {
    let mut config = video_utils_rs::PlatformVideoEncoderConfig::new(
        codec,
        width,
        height,
        time_base,
        frame_duration,
    );
    if bitrate > 0 {
        config = config.with_bitrate(u32::try_from(bitrate)?);
    }
    Ok(video_utils_rs::PlatformVideoEncoder::new(config)?)
}

#[cfg(feature = "execute")]
struct VideoProcessResult {
    media: video_utils_rs::MediaInfo,
    packets: Vec<video_utils_rs::EncodedPacket>,
    video_track_id: u32,
    input_packets: usize,
    decoded_frames: usize,
    encoded_video_packets: usize,
    copied_packets: usize,
    dropped_packets: usize,
}

#[cfg(feature = "execute")]
fn process_video_transform(
    demuxed: &video_utils_rs::DemuxedMedia,
    video_track_id: u32,
    preserve_non_video: bool,
    pipeline: &video_utils_rs::FrameTransformPipeline,
    decoder: &mut dyn VideoDecoder,
    encoder: &mut dyn VideoEncoder,
    output_codec_config: Option<Bytes>,
) -> flow_like_types::Result<VideoProcessResult> {
    let output_codec = encoder.codec_id();
    let mut output_media = demuxed.media.clone();
    let mut output_packets = Vec::<video_utils_rs::EncodedPacket>::new();
    let mut decoded_frames = 0usize;
    let mut encoded_video_packets = 0usize;
    let mut copied_packets = 0usize;
    let mut dropped_packets = 0usize;
    let mut output_dimensions = None::<(u32, u32)>;
    let mut output_time_base = None;

    for packet in &demuxed.packets {
        if packet.track_id != video_track_id {
            if preserve_non_video {
                output_packets.push(packet.clone());
                copied_packets += 1;
            } else {
                dropped_packets += 1;
            }
            continue;
        }

        for frame in decoder.decode_packet(packet)? {
            decoded_frames += 1;
            let transformed = pipeline.apply(&frame)?;
            output_dimensions = Some((transformed.width, transformed.height));
            let encoded = encoder.encode_frame(&transformed, packet.pts)?;
            if let Some(packet) = encoded.first() {
                output_time_base = Some(packet.time_base);
            }
            encoded_video_packets += encoded.len();
            output_packets.extend(encoded);
        }
    }

    for frame in decoder.flush()? {
        decoded_frames += 1;
        let transformed = pipeline.apply(&frame)?;
        output_dimensions = Some((transformed.width, transformed.height));
        let encoded = encoder.encode_frame(&transformed, 0)?;
        if let Some(packet) = encoded.first() {
            output_time_base = Some(packet.time_base);
        }
        encoded_video_packets += encoded.len();
        output_packets.extend(encoded);
    }
    let finished = encoder.finish()?;
    if let Some(packet) = finished.first() {
        output_time_base = Some(packet.time_base);
    }
    encoded_video_packets += finished.len();
    output_packets.extend(finished);

    update_video_stream(
        &mut output_media,
        video_track_id,
        output_codec,
        output_dimensions,
        output_time_base,
        output_codec_config,
        preserve_non_video,
    )?;
    sort_packets(&mut output_packets);

    Ok(VideoProcessResult {
        media: output_media,
        packets: output_packets,
        video_track_id,
        input_packets: demuxed.packets.len(),
        decoded_frames,
        encoded_video_packets,
        copied_packets,
        dropped_packets,
    })
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
fn process_subtitle_burn(
    demuxed: &video_utils_rs::DemuxedMedia,
    video_track_id: u32,
    preserve_non_video: bool,
    events: &[video_utils_rs::SubtitleEvent],
    style: &video_utils_rs::SubtitleStyle,
    decoder: &mut dyn VideoDecoder,
    encoder: &mut dyn VideoEncoder,
    output_codec_config: Option<Bytes>,
) -> flow_like_types::Result<VideoProcessResult> {
    let output_codec = encoder.codec_id();
    let mut output_media = demuxed.media.clone();
    let mut output_packets = Vec::<video_utils_rs::EncodedPacket>::new();
    let mut decoded_frames = 0usize;
    let mut encoded_video_packets = 0usize;
    let mut copied_packets = 0usize;
    let mut dropped_packets = 0usize;
    let mut output_dimensions = None::<(u32, u32)>;
    let mut output_time_base = None;

    for packet in &demuxed.packets {
        if packet.track_id != video_track_id {
            if preserve_non_video {
                output_packets.push(packet.clone());
                copied_packets += 1;
            } else {
                dropped_packets += 1;
            }
            continue;
        }

        for mut frame in decoder.decode_packet(packet)? {
            decoded_frames += 1;
            let time_ms = packet
                .time_base
                .rescale(packet.pts, video_utils_rs::TimeBase::milliseconds());
            video_utils_rs::burn_subtitles_onto_frame(&mut frame, events, time_ms, style)?;
            output_dimensions = Some((frame.width, frame.height));
            let encoded = encoder.encode_frame(&frame, packet.pts)?;
            if let Some(packet) = encoded.first() {
                output_time_base = Some(packet.time_base);
            }
            encoded_video_packets += encoded.len();
            output_packets.extend(encoded);
        }
    }

    for mut frame in decoder.flush()? {
        decoded_frames += 1;
        video_utils_rs::burn_subtitles_onto_frame(&mut frame, events, 0, style)?;
        output_dimensions = Some((frame.width, frame.height));
        let encoded = encoder.encode_frame(&frame, 0)?;
        if let Some(packet) = encoded.first() {
            output_time_base = Some(packet.time_base);
        }
        encoded_video_packets += encoded.len();
        output_packets.extend(encoded);
    }
    let finished = encoder.finish()?;
    if let Some(packet) = finished.first() {
        output_time_base = Some(packet.time_base);
    }
    encoded_video_packets += finished.len();
    output_packets.extend(finished);

    update_video_stream(
        &mut output_media,
        video_track_id,
        output_codec,
        output_dimensions,
        output_time_base,
        output_codec_config,
        preserve_non_video,
    )?;
    sort_packets(&mut output_packets);

    Ok(VideoProcessResult {
        media: output_media,
        packets: output_packets,
        video_track_id,
        input_packets: demuxed.packets.len(),
        decoded_frames,
        encoded_video_packets,
        copied_packets,
        dropped_packets,
    })
}

#[cfg(feature = "execute")]
fn update_video_stream(
    media: &mut video_utils_rs::MediaInfo,
    video_track_id: u32,
    codec: video_utils_rs::CodecId,
    dimensions: Option<(u32, u32)>,
    time_base: Option<video_utils_rs::TimeBase>,
    codec_config: Option<Bytes>,
    preserve_non_video: bool,
) -> flow_like_types::Result<()> {
    if !preserve_non_video {
        media
            .streams
            .retain(|stream| stream.track_id == video_track_id);
    }
    let stream = media
        .streams
        .iter_mut()
        .find(|stream| stream.track_id == video_track_id)
        .ok_or_else(|| flow_like_types::anyhow!("Selected video track is missing from media"))?;
    let codec_changed = stream.codec != codec;
    stream.codec = codec;
    if let Some((width, height)) = dimensions {
        stream.width = Some(width);
        stream.height = Some(height);
    }
    if let Some(time_base) = time_base {
        stream.time_base = time_base;
    }
    if let Some(codec_config) = codec_config {
        stream.codec_config = Some(codec_config);
    } else if codec_changed {
        stream.codec_config = None;
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn sort_packets(packets: &mut [video_utils_rs::EncodedPacket]) {
    packets.sort_by(|left, right| {
        let left_ts = left.time_base.ticks_to_seconds(left.decode_order_ts());
        let right_ts = right.time_base.ticks_to_seconds(right.decode_order_ts());
        left_ts
            .total_cmp(&right_ts)
            .then_with(|| left.track_id.cmp(&right.track_id))
            .then_with(|| left.pts.cmp(&right.pts))
    });
}

pub mod add_subtitle_track;
pub mod analyze_audio;
pub mod audio_to_wav;
pub mod bitstream_convert;
pub mod burn_subtitles;
pub mod check_remux_compatibility;
pub mod concat;
pub mod contact_sheet;
pub mod convert_image_format;
pub mod detect_container;
pub mod detect_silence;
pub mod encode_av1;
pub mod extract_subtitle_track;
pub mod extract_thumbnail;
pub mod extract_track;
pub mod normalize_timestamps;
pub mod package_hls_vod;
pub mod parse_subtitles;
pub mod pick_codec_backend;
pub mod probe_codec_backends;
pub mod probe_media_info;
pub mod probe_platform_codec;
pub mod remux;
pub mod shift_subtitle_file;
pub mod transcode_video;
pub mod transform_audio;
pub mod transform_image;
pub mod transform_video;
pub mod trim_keyframes;
pub mod write_subtitles;

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[test]
    fn stream_timing_stats_estimates_fps_from_packets() {
        let time_base = video_utils_rs::TimeBase::new(1, 30).unwrap();
        let stream = video_utils_rs::StreamInfo {
            track_id: 1,
            media_type: video_utils_rs::MediaType::Video,
            codec: video_utils_rs::CodecId::RawVideo,
            time_base,
            duration: Some(90),
            width: Some(1920),
            height: Some(1080),
            sample_rate: None,
            channels: None,
            language: None,
            codec_config: None,
            tags: Default::default(),
        };
        let packets = (0..90)
            .map(|pts| {
                video_utils_rs::EncodedPacket::new(
                    1,
                    video_utils_rs::CodecId::RawVideo,
                    pts,
                    1,
                    time_base,
                    Vec::<u8>::new(),
                )
            })
            .collect::<Vec<_>>();

        let stats = stream_timing_stats(&stream, &packets);

        assert_eq!(stats.frame_count, Some(90));
        assert_eq!(stats.packet_count, Some(90));
        assert_eq!(stats.fps, Some(30.0));
        assert_eq!(stats.average_frame_duration_seconds, Some(1.0 / 30.0));
    }
}
