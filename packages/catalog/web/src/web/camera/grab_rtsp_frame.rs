use std::{borrow::Cow, time::Duration};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    anyhow, async_trait, bail,
    image::{DynamicImage, RgbImage, load_from_memory},
    json::json,
};
use futures::StreamExt;
use openh264::{decoder::Decoder as H264Decoder, formats::YUVSource, nal_units};
use retina::{
    client::{
        Credentials, PlayOptions, Session, SessionOptions, SetupOptions, TcpTransportOptions,
        Transport, UdpTransportOptions,
    },
    codec::{CodecItem, ParametersRef, VideoFrame, VideoParametersCodec},
};
use rust_h265::{
    Decoder as H265Decoder, Frame as H265Frame, PixelData as H265PixelData, parse_annex_b,
};
use tokio::{runtime::Builder as TokioRuntimeBuilder, task::spawn_blocking, time::timeout};
use url::Url;

const DEFAULT_TIMEOUT_MS: u64 = 20_000;
const MIN_TIMEOUT_MS: u64 = 500;
const MAX_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_FRAMES: usize = 300;
const MIN_MAX_FRAMES: usize = 1;
const MAX_MAX_FRAMES: usize = 10_000;

struct CaptureConfig {
    rtsp_url: String,
    transport: String,
    timeout_ms: u64,
    max_frames: usize,
}

#[crate::register_node]
#[derive(Default)]
pub struct GrabRtspFrameNode {}

impl GrabRtspFrameNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GrabRtspFrameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "web_camera_grab_rtsp_frame",
            "Grab RTSP Frame",
            "Captures one frame from an RTSP camera stream",
            "Web/Camera",
        );

        node.set_long_running(true);
        node.add_icon("/flow/icons/cctv.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the RTSP frame capture",
            VariableType::Execution,
        );
        node.add_input_pin(
            "rtsp_url",
            "RTSP URL",
            "RTSP or RTSPS stream URL",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());
        node.add_input_pin(
            "transport",
            "Transport",
            "RTSP RTP transport protocol",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["tcp".to_string(), "udp".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("tcp")));
        node.add_input_pin(
            "timeout_ms",
            "Timeout",
            "Maximum time in milliseconds to connect and decode a frame",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_range((MIN_TIMEOUT_MS as f64, MAX_TIMEOUT_MS as f64))
                .build(),
        )
        .set_default_value(Some(json!(DEFAULT_TIMEOUT_MS)));
        node.add_input_pin(
            "max_frames",
            "Max Frames",
            "Maximum video frames to inspect before failing",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_range((MIN_MAX_FRAMES as f64, MAX_MAX_FRAMES as f64))
                .build(),
        )
        .set_default_value(Some(json!(DEFAULT_MAX_FRAMES)));

        node.add_output_pin(
            "exec_success",
            "Success",
            "Execution if a frame was captured",
            VariableType::Execution,
        );
        node.add_output_pin(
            "image",
            "Image",
            "The captured RTSP frame",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "exec_error",
            "Error",
            "Execution if frame capture fails",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Readable capture error",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.set_pin_value("error_message", json!("")).await?;

        let rtsp_url: String = context.evaluate_pin("rtsp_url").await?;
        let transport: String = context
            .evaluate_pin("transport")
            .await
            .unwrap_or_else(|_| "tcp".to_string());
        let timeout_ms: i64 = context
            .evaluate_pin("timeout_ms")
            .await
            .unwrap_or(DEFAULT_TIMEOUT_MS as i64);
        let max_frames: i64 = context
            .evaluate_pin("max_frames")
            .await
            .unwrap_or(DEFAULT_MAX_FRAMES as i64);

        match capture_frame(&rtsp_url, &transport, timeout_ms, max_frames).await {
            Ok(image) => {
                let node_image = NodeImage::new(context, image).await;
                context.set_pin_value("image", json!(node_image)).await?;
                context.activate_exec_pin("exec_success").await?;
            }
            Err(error) => {
                context
                    .set_pin_value("error_message", json!(error.to_string()))
                    .await?;
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }
}

async fn capture_frame(
    rtsp_url: &str,
    transport: &str,
    timeout_ms: i64,
    max_frames: i64,
) -> flow_like_types::Result<DynamicImage> {
    let config = CaptureConfig {
        rtsp_url: normalize_rtsp_url(rtsp_url)?.to_string(),
        transport: transport.to_string(),
        timeout_ms: normalize_timeout_ms(timeout_ms),
        max_frames: normalize_max_frames(max_frames),
    };

    spawn_blocking(move || run_capture_blocking(config))
        .await
        .map_err(|e| anyhow!("RTSP frame capture task failed: {e}"))?
}

fn run_capture_blocking(config: CaptureConfig) -> flow_like_types::Result<DynamicImage> {
    let runtime = TokioRuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow!("Failed to start RTSP capture runtime: {e}"))?;

    runtime.block_on(async move {
        let transport = normalize_transport(&config.transport)?;
        match timeout(
            Duration::from_millis(config.timeout_ms),
            capture_frame_inner(&config.rtsp_url, transport, config.max_frames),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => bail!("Timed out waiting for RTSP frame capture"),
        }
    })
}

async fn capture_frame_inner(
    rtsp_url: &str,
    transport: Transport,
    max_frames: usize,
) -> flow_like_types::Result<DynamicImage> {
    let (url, credentials) = parse_rtsp_url(rtsp_url)?;
    let mut session_options =
        SessionOptions::default().user_agent("FlowLike RTSP Frame Grabber".to_string());

    if credentials.is_some() {
        session_options = session_options.creds(credentials);
    }

    let mut session = Session::describe(url, session_options)
        .await
        .map_err(|e| anyhow!("RTSP DESCRIBE failed: {e}"))?;

    let stream_i = session
        .streams()
        .iter()
        .position(|stream| stream.media().eq_ignore_ascii_case("video"))
        .ok_or_else(|| anyhow!("RTSP stream did not advertise a video track"))?;

    let mut decoder = FrameDecoder::for_stream(&session.streams()[stream_i])?;
    let setup_options = SetupOptions::default().transport(transport);

    session
        .setup(stream_i, setup_options)
        .await
        .map_err(|e| anyhow!("RTSP SETUP failed: {e}"))?;

    let playing = session
        .play(PlayOptions::default())
        .await
        .map_err(|e| anyhow!("RTSP PLAY failed: {e}"))?;
    let mut frames = playing
        .demuxed()
        .map_err(|e| anyhow!("RTSP stream cannot be depacketized: {e}"))?;

    let mut seen_video_frames = 0usize;
    while let Some(item) = frames.next().await {
        let item = item.map_err(|e| anyhow!("RTSP stream read failed: {e}"))?;
        let CodecItem::VideoFrame(frame) = item else {
            continue;
        };

        if frame.stream_id() != stream_i {
            continue;
        }

        seen_video_frames += 1;
        if frame.loss() > 0 {
            if seen_video_frames >= max_frames {
                bail!(
                    "Decoded no {} image after {seen_video_frames} video frames",
                    decoder.codec_name()
                );
            }
            continue;
        }

        if let Some(image) = decoder.decode(frame)? {
            return Ok(image);
        }

        if seen_video_frames >= max_frames {
            bail!(
                "Decoded no {} image after {seen_video_frames} video frames",
                decoder.codec_name()
            );
        }
    }

    bail!("RTSP stream ended before a decodable video frame was received")
}

fn normalize_rtsp_url(value: &str) -> flow_like_types::Result<&str> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("rtsp://") || lower.starts_with("rtsps://") {
        return Ok(trimmed);
    }

    bail!("RTSP URL must start with rtsp:// or rtsps://")
}

fn normalize_transport(value: &str) -> flow_like_types::Result<Transport> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "tcp" => Ok(Transport::Tcp(TcpTransportOptions::default())),
        "udp" => Ok(Transport::Udp(UdpTransportOptions::default())),
        other => bail!("Unsupported RTSP transport: {other}"),
    }
}

fn normalize_timeout_ms(value: i64) -> u64 {
    if value <= 0 {
        return DEFAULT_TIMEOUT_MS;
    }

    (value as u64).clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
}

fn normalize_max_frames(value: i64) -> usize {
    if value <= 0 {
        return DEFAULT_MAX_FRAMES;
    }

    (value as usize).clamp(MIN_MAX_FRAMES, MAX_MAX_FRAMES)
}

fn parse_rtsp_url(value: &str) -> flow_like_types::Result<(Url, Option<Credentials>)> {
    let mut url = Url::parse(value)?;
    let credentials = if url.username().is_empty() {
        None
    } else {
        let credentials = Credentials {
            username: url.username().to_string(),
            password: url.password().unwrap_or_default().to_string(),
        };

        url.set_username("")
            .map_err(|_| anyhow!("Failed to remove RTSP username from URL"))?;
        url.set_password(None)
            .map_err(|_| anyhow!("Failed to remove RTSP password from URL"))?;

        Some(credentials)
    };

    Ok((url, credentials))
}

enum FrameDecoder {
    H264(H264FrameDecoder),
    H265(H265FrameDecoder),
    Jpeg,
}

struct H264FrameDecoder {
    decoder: H264Decoder,
}

struct H265FrameDecoder {
    decoder: H265Decoder,
}

impl FrameDecoder {
    fn for_stream(stream: &retina::client::Stream) -> flow_like_types::Result<Self> {
        if let Some(ParametersRef::Video(parameters)) = stream.parameters() {
            return match parameters.codec_params() {
                VideoParametersCodec::H264 { sps, pps } => {
                    Ok(Self::H264(H264FrameDecoder::new(vec![
                        sps.to_vec(),
                        pps.to_vec(),
                    ])?))
                }
                VideoParametersCodec::H265 { vps, sps, pps } => {
                    Ok(Self::H265(H265FrameDecoder::new(vec![
                        vps.to_vec(),
                        sps.to_vec(),
                        pps.to_vec(),
                    ])))
                }
                VideoParametersCodec::Jpeg => Ok(Self::Jpeg),
                _ => bail!(
                    "Unsupported RTSP video codec: {}",
                    parameters.rfc6381_codec()
                ),
            };
        }

        match stream.encoding_name().trim().to_ascii_lowercase().as_str() {
            "h264" => Ok(Self::H264(H264FrameDecoder::new(Vec::new())?)),
            "h265" | "hevc" => Ok(Self::H265(H265FrameDecoder::new(Vec::new()))),
            "jpeg" | "mjpeg" => Ok(Self::Jpeg),
            encoding => bail!("Unsupported RTSP video encoding: {encoding}"),
        }
    }

    fn codec_name(&self) -> &'static str {
        match self {
            Self::H264(_) => "H.264",
            Self::H265(_) => "H.265",
            Self::Jpeg => "JPEG",
        }
    }

    fn decode(&mut self, frame: VideoFrame) -> flow_like_types::Result<Option<DynamicImage>> {
        match self {
            Self::H264(decoder) => decode_h264_frame(decoder, frame.data()),
            Self::H265(decoder) => decode_h265_frame(decoder, frame.data()),
            Self::Jpeg => {
                Ok(Some(load_from_memory(frame.data()).map_err(|e| {
                    anyhow!("Failed to decode RTSP JPEG frame: {e}")
                })?))
            }
        }
    }
}

impl H264FrameDecoder {
    fn new(parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
        let mut decoder = H264Decoder::new()?;
        for parameter_set in parameter_sets {
            let _ = decoder.decode(&parameter_set);
        }
        Ok(Self { decoder })
    }
}

impl H265FrameDecoder {
    fn new(parameter_sets: Vec<Vec<u8>>) -> Self {
        let mut decoder = H265Decoder::new();
        for parameter_set in parameter_sets {
            decode_h265_parameter_set(&mut decoder, &parameter_set);
        }
        Self { decoder }
    }
}

fn decode_h264_frame(
    decoder: &mut H264FrameDecoder,
    data: &[u8],
) -> flow_like_types::Result<Option<DynamicImage>> {
    let annex_b = h26x_payload_to_annex_b(data)?;

    for packet in nal_units(&annex_b) {
        match decoder.decoder.decode(packet) {
            Ok(Some(decoded)) => return Ok(Some(decoded_yuv_to_image(&decoded)?)),
            Ok(None) => {}
            Err(_) => {}
        }
    }

    Ok(None)
}

fn decode_h265_frame(
    decoder: &mut H265FrameDecoder,
    data: &[u8],
) -> flow_like_types::Result<Option<DynamicImage>> {
    let annex_b = h26x_payload_to_annex_b(data)?;
    let nals = parse_annex_b(&annex_b);

    if nals.is_empty() {
        return Ok(None);
    }

    for nal in nals {
        match decoder.decoder.decode_nal(&nal) {
            Ok(Some(frame)) => return h265_frame_to_image(&frame).map(Some),
            Ok(None) => {}
            Err(_) => {}
        }
    }

    Ok(None)
}

fn decode_h265_parameter_set(decoder: &mut H265Decoder, parameter_set: &[u8]) {
    let mut annex_b = Vec::with_capacity(parameter_set.len() + 4);
    annex_b.extend_from_slice(&[0, 0, 0, 1]);
    annex_b.extend_from_slice(parameter_set);

    for nal in parse_annex_b(&annex_b) {
        let _ = decoder.decode_nal(&nal);
    }
}

fn h26x_payload_to_annex_b(data: &[u8]) -> flow_like_types::Result<Cow<'_, [u8]>> {
    if data.is_empty() {
        return Ok(Cow::Borrowed(data));
    }

    if let Some(annex_b) = four_byte_length_prefixed_to_annex_b(data)? {
        return Ok(Cow::Owned(annex_b));
    }

    if starts_with_annex_b_start_code(data) {
        return Ok(Cow::Borrowed(data));
    }

    let mut single_nal = Vec::with_capacity(data.len() + 4);
    single_nal.extend_from_slice(&[0, 0, 0, 1]);
    single_nal.extend_from_slice(data);
    Ok(Cow::Owned(single_nal))
}

fn starts_with_annex_b_start_code(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1])
}

fn four_byte_length_prefixed_to_annex_b(data: &[u8]) -> flow_like_types::Result<Option<Vec<u8>>> {
    let mut offset = 0usize;
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut nal_count = 0usize;

    while offset < data.len() {
        if data.len().saturating_sub(offset) < 4 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        if len == 0 {
            continue;
        }

        if data.len().saturating_sub(offset) < len {
            return Ok(None);
        }

        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[offset..offset + len]);
        offset += len;
        nal_count += 1;
    }

    Ok((nal_count > 0).then_some(out))
}

fn decoded_yuv_to_image(
    decoded: &openh264::decoder::DecodedYUV<'_>,
) -> flow_like_types::Result<DynamicImage> {
    let (width, height) = decoded.dimensions();
    let mut rgb = vec![0; decoded.rgb8_len()];
    decoded.write_rgb8(&mut rgb);

    let image = RgbImage::from_raw(width as u32, height as u32, rgb)
        .ok_or_else(|| anyhow!("Decoded H.264 frame had invalid RGB dimensions"))?;

    Ok(DynamicImage::ImageRgb8(image))
}

fn h265_frame_to_image(frame: &H265Frame) -> flow_like_types::Result<DynamicImage> {
    match (&frame.y, &frame.u, &frame.v) {
        (H265PixelData::U8(y), H265PixelData::U8(u), H265PixelData::U8(v)) => {
            yuv420_to_rgb_image(frame.width, frame.height, 8, y, u, v)
        }
        (H265PixelData::U16(y), H265PixelData::U16(u), H265PixelData::U16(v)) => {
            let y = scale_plane_u16_to_u8(y, frame.bit_depth);
            let u = scale_plane_u16_to_u8(u, frame.bit_depth);
            let v = scale_plane_u16_to_u8(v, frame.bit_depth);
            yuv420_to_rgb_image(frame.width, frame.height, 8, &y, &u, &v)
        }
        _ => bail!("Decoded H.265 frame used mixed pixel plane types"),
    }
}

fn scale_plane_u16_to_u8(plane: &[u16], bit_depth: u8) -> Vec<u8> {
    let max_value = ((1u32 << bit_depth.min(16)) - 1).max(1);
    plane
        .iter()
        .map(|sample| (((*sample as u32) * 255 + (max_value / 2)) / max_value) as u8)
        .collect()
}

fn yuv420_to_rgb_image(
    width: u32,
    height: u32,
    bit_depth: u8,
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
) -> flow_like_types::Result<DynamicImage> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    let luma_len = width_usize
        .checked_mul(height_usize)
        .ok_or_else(|| anyhow!("Decoded H.265 frame dimensions overflowed"))?;
    let chroma_width = width_usize.div_ceil(2).max(1);
    let chroma_height = height_usize.div_ceil(2).max(1);
    let chroma_len = chroma_width
        .checked_mul(chroma_height)
        .ok_or_else(|| anyhow!("Decoded H.265 chroma dimensions overflowed"))?;

    if y_plane.len() < luma_len || u_plane.len() < chroma_len || v_plane.len() < chroma_len {
        bail!("Decoded H.265 frame had incomplete YUV420 planes");
    }

    let mut rgb = Vec::with_capacity(luma_len * 3);
    let neutral_chroma = 1i32 << bit_depth.saturating_sub(1);

    for y in 0..height_usize {
        for x in 0..width_usize {
            let y_sample = y_plane[y * width_usize + x] as i32;
            let chroma_index = (y / 2) * chroma_width + (x / 2);
            let cb = u_plane[chroma_index] as i32 - neutral_chroma;
            let cr = v_plane[chroma_index] as i32 - neutral_chroma;

            let r = y_sample + ((359 * cr) >> 8);
            let g = y_sample - ((88 * cb + 183 * cr) >> 8);
            let b = y_sample + ((454 * cb) >> 8);

            rgb.push(clamp_u8(r));
            rgb.push(clamp_u8(g));
            rgb.push(clamp_u8(b));
        }
    }

    let image = RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| anyhow!("Decoded H.265 frame had invalid RGB dimensions"))?;

    Ok(DynamicImage::ImageRgb8(image))
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_rtsp_url() {
        assert_eq!(
            normalize_rtsp_url(" rtsp://example.test/live ").unwrap(),
            "rtsp://example.test/live"
        );
        assert_eq!(
            normalize_rtsp_url("rtsps://example.test/live").unwrap(),
            "rtsps://example.test/live"
        );
        assert!(normalize_rtsp_url("http://example.test/live").is_err());
    }

    #[test]
    fn normalizes_transport() {
        assert!(matches!(
            normalize_transport("").unwrap(),
            Transport::Tcp(_)
        ));
        assert!(matches!(
            normalize_transport("TCP").unwrap(),
            Transport::Tcp(_)
        ));
        assert!(matches!(
            normalize_transport("udp").unwrap(),
            Transport::Udp(_)
        ));
        assert!(normalize_transport("file").is_err());
    }

    #[test]
    fn clamps_timeout() {
        assert_eq!(normalize_timeout_ms(0), DEFAULT_TIMEOUT_MS);
        assert_eq!(normalize_timeout_ms(100), MIN_TIMEOUT_MS);
        assert_eq!(normalize_timeout_ms(3_000), 3_000);
        assert_eq!(normalize_timeout_ms(500_000), MAX_TIMEOUT_MS);
    }

    #[test]
    fn converts_four_byte_length_prefixed_h26x_to_annex_b() {
        let data = [0, 0, 0, 2, 0x65, 0x88, 0, 0, 0, 1, 0x06];
        let annex_b = h26x_payload_to_annex_b(&data).unwrap();
        assert_eq!(
            annex_b.as_ref(),
            &[0, 0, 0, 1, 0x65, 0x88, 0, 0, 0, 1, 0x06]
        );
    }

    #[test]
    fn keeps_annex_b_h26x_payloads() {
        let data = [0, 0, 0, 1, 0x65, 0x88];
        let annex_b = h26x_payload_to_annex_b(&data).unwrap();
        assert_eq!(annex_b.as_ref(), data);
    }
}
