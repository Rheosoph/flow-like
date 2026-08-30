#[cfg(feature = "execute")]
use std::{borrow::Cow, time::Duration};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
#[cfg(all(feature = "execute", any(target_os = "macos", target_os = "ios")))]
use flow_like_types::image::RgbaImage;
#[cfg(feature = "execute")]
use flow_like_types::{
    anyhow, bail,
    image::{DynamicImage, RgbImage, load_from_memory},
};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::StreamExt;
#[cfg(feature = "execute")]
use openh264::{decoder::Decoder as H264Decoder, formats::YUVSource, nal_units};
#[cfg(feature = "execute")]
use retina::{
    client::{
        Credentials, PlayOptions, Session, SessionOptions, SetupOptions, TcpTransportOptions,
        Transport, UdpTransportOptions,
    },
    codec::{CodecItem, ParametersRef, VideoFrame, VideoParametersCodec},
};
#[cfg(feature = "execute")]
use tokio::time::timeout;
#[cfg(feature = "execute")]
use url::Url;

const DEFAULT_TIMEOUT_MS: u64 = 20_000;
const MIN_TIMEOUT_MS: u64 = 500;
const MAX_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_FRAMES: usize = 300;
const MIN_MAX_FRAMES: usize = 1;
const MAX_MAX_FRAMES: usize = 10_000;

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
        node.set_flowscript_name("camera", "grabRtspFrame");

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

    #[cfg(feature = "execute")]
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

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RTSP frame capture requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn capture_frame(
    rtsp_url: &str,
    transport: &str,
    timeout_ms: i64,
    max_frames: i64,
) -> flow_like_types::Result<DynamicImage> {
    let transport = normalize_transport(transport)?;
    let timeout_ms = normalize_timeout_ms(timeout_ms);
    let max_frames = normalize_max_frames(max_frames);
    let rtsp_url = normalize_rtsp_url(rtsp_url)?.to_string();

    match timeout(
        Duration::from_millis(timeout_ms),
        capture_frame_inner(&rtsp_url, transport, max_frames),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => bail!("Timed out waiting for RTSP frame capture"),
    }
}

#[cfg(feature = "execute")]
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

#[cfg(feature = "execute")]
fn normalize_rtsp_url(value: &str) -> flow_like_types::Result<&str> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("rtsp://") || lower.starts_with("rtsps://") {
        return Ok(trimmed);
    }

    bail!("RTSP URL must start with rtsp:// or rtsps://")
}

#[cfg(feature = "execute")]
fn normalize_transport(value: &str) -> flow_like_types::Result<Transport> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "tcp" => Ok(Transport::Tcp(TcpTransportOptions::default())),
        "udp" => Ok(Transport::Udp(UdpTransportOptions::default())),
        other => bail!("Unsupported RTSP transport: {other}"),
    }
}

#[cfg(feature = "execute")]
fn normalize_timeout_ms(value: i64) -> u64 {
    if value <= 0 {
        return DEFAULT_TIMEOUT_MS;
    }

    (value as u64).clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
}

#[cfg(feature = "execute")]
fn normalize_max_frames(value: i64) -> usize {
    if value <= 0 {
        return DEFAULT_MAX_FRAMES;
    }

    (value as usize).clamp(MIN_MAX_FRAMES, MAX_MAX_FRAMES)
}

#[cfg(feature = "execute")]
fn parse_rtsp_url(value: &str) -> flow_like_types::Result<(Url, Option<Credentials>)> {
    let mut url = Url::parse(value)?;
    let credentials = if url.username().is_empty() {
        None
    } else {
        let username = urlencoding::decode(url.username())
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| url.username().to_string());
        let password = url.password().unwrap_or_default();
        let password = urlencoding::decode(password)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| password.to_string());

        let credentials = Credentials { username, password };

        url.set_username("")
            .map_err(|_| anyhow!("Failed to remove RTSP username from URL"))?;
        url.set_password(None)
            .map_err(|_| anyhow!("Failed to remove RTSP password from URL"))?;

        Some(credentials)
    };

    Ok((url, credentials))
}

#[cfg(feature = "execute")]
enum FrameDecoder {
    H264(H264FrameDecoder),
    Hevc(PlatformHevcFrameDecoder),
    Jpeg,
}

#[cfg(feature = "execute")]
struct H264FrameDecoder {
    decoder: H264Decoder,
}

#[cfg(feature = "execute")]
struct PlatformHevcFrameDecoder {
    decoder: platform_hevc::Decoder,
}

#[cfg(feature = "execute")]
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
                    Ok(Self::Hevc(PlatformHevcFrameDecoder::new(vec![
                        vps.to_vec(),
                        sps.to_vec(),
                        pps.to_vec(),
                    ])?))
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
            "h265" | "hevc" => Ok(Self::Hevc(PlatformHevcFrameDecoder::new(Vec::new())?)),
            "jpeg" | "mjpeg" => Ok(Self::Jpeg),
            encoding => bail!("Unsupported RTSP video encoding: {encoding}"),
        }
    }

    fn codec_name(&self) -> &'static str {
        match self {
            Self::H264(_) => "H.264",
            Self::Hevc(_) => "H.265/HEVC",
            Self::Jpeg => "JPEG",
        }
    }

    fn decode(&mut self, frame: VideoFrame) -> flow_like_types::Result<Option<DynamicImage>> {
        match self {
            Self::H264(decoder) => decode_h264_frame(decoder, frame.data()),
            Self::Hevc(decoder) => decode_platform_hevc_frame(decoder, frame.data()),
            Self::Jpeg => {
                Ok(Some(load_from_memory(frame.data()).map_err(|e| {
                    anyhow!("Failed to decode RTSP JPEG frame: {e}")
                })?))
            }
        }
    }
}

#[cfg(feature = "execute")]
impl H264FrameDecoder {
    fn new(parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
        let mut decoder = H264Decoder::new()?;
        for parameter_set in parameter_sets {
            let _ = decoder.decode(&parameter_set);
        }
        Ok(Self { decoder })
    }
}

#[cfg(feature = "execute")]
impl PlatformHevcFrameDecoder {
    fn new(parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
        let decoder = platform_hevc::Decoder::new(parameter_sets)?;
        Ok(Self { decoder })
    }
}

#[cfg(feature = "execute")]
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

#[cfg(feature = "execute")]
fn decode_platform_hevc_frame(
    decoder: &mut PlatformHevcFrameDecoder,
    data: &[u8],
) -> flow_like_types::Result<Option<DynamicImage>> {
    decoder.decoder.decode(data)
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug, Default)]
struct HevcParameterSets {
    vps: Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

#[cfg(feature = "execute")]
impl HevcParameterSets {
    fn from_parameter_sets(parameter_sets: Vec<Vec<u8>>) -> Self {
        let mut sets = Self::default();
        for parameter_set in parameter_sets {
            if parameter_set.is_empty() {
                continue;
            }

            if starts_with_annex_b_start_code(&parameter_set) {
                if let Ok(annex_b) = h26x_payload_to_annex_b(&parameter_set) {
                    sets.update_from_nals(&annex_b_nals(&annex_b));
                }
            } else {
                sets.update_from_nal(parameter_set);
            }
        }
        sets
    }

    fn update_from_nals(&mut self, nals: &[&[u8]]) {
        for nal in nals {
            self.update_from_nal((*nal).to_vec());
        }
    }

    fn update_from_nal(&mut self, nal: Vec<u8>) {
        match hevc_nal_unit_type(&nal) {
            Some(32) => self.vps = Some(nal),
            Some(33) => self.sps = Some(nal),
            Some(34) => self.pps = Some(nal),
            _ => {}
        }
    }

    fn complete(&self) -> bool {
        self.vps.is_some() && self.sps.is_some() && self.pps.is_some()
    }

    #[cfg(any(target_os = "android", target_os = "windows"))]
    fn to_annex_b(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in [&self.vps, &self.sps, &self.pps].into_iter().flatten() {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    #[cfg(any(target_os = "android", target_os = "windows"))]
    fn dimensions(&self) -> flow_like_types::Result<Option<HevcDimensions>> {
        self.sps
            .as_deref()
            .map(parse_hevc_sps_dimensions)
            .transpose()
    }
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HevcDimensions {
    coded_width: u32,
    coded_height: u32,
    display_width: u32,
    display_height: u32,
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
fn has_decodable_hevc_nal(nals: &[&[u8]]) -> bool {
    nals.iter().any(|nal| {
        !matches!(
            hevc_nal_unit_type(nal),
            Some(32 | 33 | 34 | 35 | 39 | 40) | None
        )
    })
}

#[cfg(all(feature = "execute", any(target_os = "macos", target_os = "ios")))]
fn hevc_nal_is_parameter_set(nal: &[u8]) -> bool {
    matches!(hevc_nal_unit_type(nal), Some(32..=34))
}

#[cfg(feature = "execute")]
fn hevc_nal_unit_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|byte| (byte >> 1) & 0x3f)
}

#[cfg(all(feature = "execute", any(target_os = "macos", target_os = "ios")))]
fn nals_to_four_byte_length_prefixed(nals: &[&[u8]]) -> flow_like_types::Result<Vec<u8>> {
    let mut out = Vec::new();
    for nal in nals {
        if nal.is_empty() || hevc_nal_is_parameter_set(nal) {
            continue;
        }

        let len = u32::try_from(nal.len())
            .map_err(|_| anyhow!("HEVC NAL unit is too large for platform decoder"))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nal);
    }
    Ok(out)
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
fn parse_hevc_sps_dimensions(nal: &[u8]) -> flow_like_types::Result<HevcDimensions> {
    let rbsp = hevc_nal_rbsp(nal)?;
    let mut bits = BitReader::new(&rbsp);

    bits.skip_bits(4)?;
    let sps_max_sub_layers_minus1 = bits.read_bits(3)? as u8;
    bits.skip_bits(1)?;
    skip_hevc_profile_tier_level(&mut bits, true, sps_max_sub_layers_minus1)?;
    bits.read_ue()?;
    let chroma_format_idc = bits.read_ue()?;
    let separate_colour_plane_flag = if chroma_format_idc == 3 {
        bits.read_bool()?
    } else {
        false
    };
    let coded_width = bits.read_ue()?;
    let coded_height = bits.read_ue()?;

    let mut display_width = coded_width;
    let mut display_height = coded_height;
    if bits.read_bool()? {
        let conf_win_left_offset = bits.read_ue()?;
        let conf_win_right_offset = bits.read_ue()?;
        let conf_win_top_offset = bits.read_ue()?;
        let conf_win_bottom_offset = bits.read_ue()?;
        let (sub_width_c, sub_height_c) =
            hevc_chroma_subsampling(chroma_format_idc, separate_colour_plane_flag)?;
        let crop_width = conf_win_left_offset
            .checked_add(conf_win_right_offset)
            .and_then(|value| value.checked_mul(sub_width_c))
            .ok_or_else(|| anyhow!("HEVC SPS conformance window width overflowed"))?;
        let crop_height = conf_win_top_offset
            .checked_add(conf_win_bottom_offset)
            .and_then(|value| value.checked_mul(sub_height_c))
            .ok_or_else(|| anyhow!("HEVC SPS conformance window height overflowed"))?;
        display_width = display_width
            .checked_sub(crop_width)
            .ok_or_else(|| anyhow!("HEVC SPS conformance window exceeds coded width"))?;
        display_height = display_height
            .checked_sub(crop_height)
            .ok_or_else(|| anyhow!("HEVC SPS conformance window exceeds coded height"))?;
    }

    if coded_width == 0 || coded_height == 0 || display_width == 0 || display_height == 0 {
        bail!("HEVC SPS advertised an empty frame size");
    }

    Ok(HevcDimensions {
        coded_width,
        coded_height,
        display_width,
        display_height,
    })
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
fn hevc_nal_rbsp(nal: &[u8]) -> flow_like_types::Result<Vec<u8>> {
    let nals = if starts_with_annex_b_start_code(nal) {
        annex_b_nals(nal)
    } else {
        vec![nal]
    };
    let sps = nals
        .into_iter()
        .find(|candidate| hevc_nal_unit_type(candidate) == Some(33))
        .ok_or_else(|| anyhow!("HEVC SPS parameter set was not present"))?;

    if sps.len() < 3 {
        bail!("HEVC SPS parameter set is too short");
    }

    Ok(remove_emulation_prevention_bytes(&sps[2..]))
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
fn remove_emulation_prevention_bytes(data: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(data.len());
    let mut i = 0usize;
    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 3 {
            rbsp.extend_from_slice(&[0, 0]);
            i += 3;
        } else {
            rbsp.push(data[i]);
            i += 1;
        }
    }
    rbsp
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
fn skip_hevc_profile_tier_level(
    bits: &mut BitReader<'_>,
    profile_present_flag: bool,
    max_sub_layers_minus1: u8,
) -> flow_like_types::Result<()> {
    if profile_present_flag {
        bits.skip_bits(88)?;
    }
    bits.skip_bits(8)?;

    let mut sub_layer_profile_present = [false; 8];
    let mut sub_layer_level_present = [false; 8];
    for i in 0..max_sub_layers_minus1 as usize {
        sub_layer_profile_present[i] = bits.read_bool()?;
        sub_layer_level_present[i] = bits.read_bool()?;
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            bits.skip_bits(2)?;
        }
    }

    for i in 0..max_sub_layers_minus1 as usize {
        if sub_layer_profile_present[i] {
            bits.skip_bits(88)?;
        }
        if sub_layer_level_present[i] {
            bits.skip_bits(8)?;
        }
    }

    Ok(())
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
fn hevc_chroma_subsampling(
    chroma_format_idc: u32,
    separate_colour_plane_flag: bool,
) -> flow_like_types::Result<(u32, u32)> {
    if separate_colour_plane_flag {
        return Ok((1, 1));
    }

    match chroma_format_idc {
        0 => Ok((1, 1)),
        1 => Ok((2, 2)),
        2 => Ok((2, 1)),
        3 => Ok((1, 1)),
        _ => bail!("Unsupported HEVC chroma_format_idc: {chroma_format_idc}"),
    }
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
struct BitReader<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

#[cfg(all(feature = "execute", any(target_os = "android", target_os = "windows")))]
impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bool(&mut self) -> flow_like_types::Result<bool> {
        Ok(self.read_bits(1)? != 0)
    }

    fn read_bits(&mut self, count: usize) -> flow_like_types::Result<u32> {
        if count > 32 {
            bail!("HEVC bit reader cannot read more than 32 bits at once");
        }

        let mut value = 0u32;
        for _ in 0..count {
            let byte = self
                .data
                .get(self.bit_offset / 8)
                .ok_or_else(|| anyhow!("Unexpected end of HEVC SPS bitstream"))?;
            let bit = (byte >> (7 - (self.bit_offset % 8))) & 1;
            value = (value << 1) | u32::from(bit);
            self.bit_offset += 1;
        }
        Ok(value)
    }

    fn skip_bits(&mut self, count: usize) -> flow_like_types::Result<()> {
        let new_offset = self
            .bit_offset
            .checked_add(count)
            .ok_or_else(|| anyhow!("HEVC bit reader offset overflowed"))?;
        if new_offset > self.data.len() * 8 {
            bail!("Unexpected end of HEVC SPS bitstream");
        }
        self.bit_offset = new_offset;
        Ok(())
    }

    fn read_ue(&mut self) -> flow_like_types::Result<u32> {
        let mut leading_zero_bits = 0usize;
        while !self.read_bool()? {
            leading_zero_bits += 1;
            if leading_zero_bits > 31 {
                bail!("HEVC exponential-Golomb value is too large");
            }
        }

        let suffix = if leading_zero_bits == 0 {
            0
        } else {
            self.read_bits(leading_zero_bits)?
        };

        Ok(((1u32 << leading_zero_bits) - 1) + suffix)
    }
}

#[cfg(all(feature = "execute", target_os = "linux"))]
fn platform_target_name() -> &'static str {
    "Linux"
}

#[cfg(all(
    feature = "execute",
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "windows",
        target_os = "android",
        target_os = "linux"
    ))
))]
fn platform_target_name() -> &'static str {
    "this platform"
}

#[cfg(all(feature = "execute", target_os = "linux"))]
fn platform_hevc_backend_name() -> &'static str {
    "VA-API/NVDEC"
}

#[cfg(all(
    feature = "execute",
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "windows",
        target_os = "android",
        target_os = "linux"
    ))
))]
fn platform_hevc_backend_name() -> &'static str {
    "no known platform HEVC backend"
}

#[cfg(all(feature = "execute", any(target_os = "macos", target_os = "ios")))]
mod platform_hevc {
    use super::*;
    use std::{
        ffi::c_void,
        panic::{AssertUnwindSafe, catch_unwind},
        ptr,
    };

    type CFAllocatorRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFIndex = isize;
    type CFNumberRef = *const c_void;
    type CFNumberType = CFIndex;
    type CFStringRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CMBlockBufferRef = *mut c_void;
    type CMFormatDescriptionRef = *mut c_void;
    type CMItemCount = isize;
    type CMSampleBufferRef = *mut c_void;
    type CMVideoFormatDescriptionRef = *mut c_void;
    type CVOptionFlags = u64;
    type CVPixelBufferRef = *mut c_void;
    type OSStatus = i32;
    type VTDecodeFrameFlags = u32;
    type VTDecodeInfoFlags = u32;
    type VTDecompressionSessionRef = *mut c_void;

    const K_CF_NUMBER_SINT32_TYPE: CFNumberType = 3;
    const K_CV_PIXEL_BUFFER_LOCK_READ_ONLY: CVOptionFlags = 1;
    const K_CV_PIXEL_FORMAT_TYPE_32_BGRA: u32 = 0x4247_5241;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    impl CMTime {
        const INVALID: Self = Self {
            value: 0,
            timescale: 0,
            flags: 0,
            epoch: 0,
        };

        const ZERO: Self = Self {
            value: 0,
            timescale: 1,
            flags: 1,
            epoch: 0,
        };
    }

    #[repr(C)]
    struct CMSampleTimingInfo {
        duration: CMTime,
        presentation_time_stamp: CMTime,
        decode_time_stamp: CMTime,
    }

    #[repr(C)]
    struct VTDecompressionOutputCallbackRecord {
        decompression_output_callback: Option<
            extern "C" fn(
                decompression_output_ref_con: *mut c_void,
                source_frame_ref_con: *mut c_void,
                status: OSStatus,
                info_flags: VTDecodeInfoFlags,
                image_buffer: CVPixelBufferRef,
                presentation_time_stamp: CMTime,
                presentation_duration: CMTime,
            ),
        >,
        decompression_output_ref_con: *mut c_void,
    }

    #[allow(clippy::duplicated_attributes)]
    #[link(name = "CoreFoundation", kind = "framework")]
    #[link(name = "CoreMedia", kind = "framework")]
    #[link(name = "CoreVideo", kind = "framework")]
    #[link(name = "VideoToolbox", kind = "framework")]
    unsafe extern "C" {
        static kCVPixelBufferPixelFormatTypeKey: CFStringRef;

        fn CFDictionaryCreate(
            allocator: CFAllocatorRef,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: CFIndex,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFDictionaryRef;
        fn CFNumberCreate(
            allocator: CFAllocatorRef,
            the_type: CFNumberType,
            value_ptr: *const c_void,
        ) -> CFNumberRef;
        fn CFRelease(cf: CFTypeRef);

        fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
            allocator: CFAllocatorRef,
            parameter_set_count: usize,
            parameter_set_pointers: *const *const u8,
            parameter_set_sizes: *const usize,
            nal_unit_header_length: i32,
            extensions: CFDictionaryRef,
            format_description_out: *mut CMVideoFormatDescriptionRef,
        ) -> OSStatus;
        fn CMBlockBufferCreateWithMemoryBlock(
            structure_allocator: CFAllocatorRef,
            memory_block: *mut c_void,
            block_length: usize,
            block_allocator: CFAllocatorRef,
            custom_block_source: *const c_void,
            offset_to_data: usize,
            data_length: usize,
            flags: u32,
            block_buffer_out: *mut CMBlockBufferRef,
        ) -> OSStatus;
        fn CMBlockBufferReplaceDataBytes(
            source_bytes: *const c_void,
            destination_buffer: CMBlockBufferRef,
            offset_into_destination: usize,
            data_length: usize,
        ) -> OSStatus;
        fn CMSampleBufferCreateReady(
            allocator: CFAllocatorRef,
            data_buffer: CMBlockBufferRef,
            format_description: CMFormatDescriptionRef,
            num_samples: CMItemCount,
            num_sample_timing_entries: CMItemCount,
            sample_timing_array: *const CMSampleTimingInfo,
            num_sample_size_entries: CMItemCount,
            sample_size_array: *const usize,
            sample_buffer_out: *mut CMSampleBufferRef,
        ) -> OSStatus;

        fn CVPixelBufferLockBaseAddress(
            pixel_buffer: CVPixelBufferRef,
            lock_flags: CVOptionFlags,
        ) -> OSStatus;
        fn CVPixelBufferUnlockBaseAddress(
            pixel_buffer: CVPixelBufferRef,
            unlock_flags: CVOptionFlags,
        ) -> OSStatus;
        fn CVPixelBufferGetPixelFormatType(pixel_buffer: CVPixelBufferRef) -> u32;
        fn CVPixelBufferGetWidth(pixel_buffer: CVPixelBufferRef) -> usize;
        fn CVPixelBufferGetHeight(pixel_buffer: CVPixelBufferRef) -> usize;
        fn CVPixelBufferGetBytesPerRow(pixel_buffer: CVPixelBufferRef) -> usize;
        fn CVPixelBufferGetBaseAddress(pixel_buffer: CVPixelBufferRef) -> *mut c_void;

        fn VTDecompressionSessionCreate(
            allocator: CFAllocatorRef,
            video_format_description: CMVideoFormatDescriptionRef,
            video_decoder_specification: CFDictionaryRef,
            destination_image_buffer_attributes: CFDictionaryRef,
            output_callback: *const VTDecompressionOutputCallbackRecord,
            decompression_session_out: *mut VTDecompressionSessionRef,
        ) -> OSStatus;
        fn VTDecompressionSessionDecodeFrame(
            session: VTDecompressionSessionRef,
            sample_buffer: CMSampleBufferRef,
            decode_flags: VTDecodeFrameFlags,
            source_frame_ref_con: *mut c_void,
            info_flags_out: *mut VTDecodeInfoFlags,
        ) -> OSStatus;
        fn VTDecompressionSessionWaitForAsynchronousFrames(
            session: VTDecompressionSessionRef,
        ) -> OSStatus;
        fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);
    }

    pub struct Decoder {
        parameter_sets: HevcParameterSets,
        session: Option<AppleHevcSession>,
    }

    // SAFETY: The decoder owns its CoreFoundation/VideoToolbox references and only
    // exposes mutable decode access, so moving it between executor threads does not
    // create concurrent access to the underlying session.
    unsafe impl Send for Decoder {}

    impl Decoder {
        pub fn new(parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
            Ok(Self {
                parameter_sets: HevcParameterSets::from_parameter_sets(parameter_sets),
                session: None,
            })
        }

        pub fn decode(&mut self, data: &[u8]) -> flow_like_types::Result<Option<DynamicImage>> {
            let annex_b = h26x_payload_to_annex_b(data)?;
            let nals = annex_b_nals(&annex_b);
            if nals.is_empty() {
                return Ok(None);
            }

            self.parameter_sets.update_from_nals(&nals);

            let sample = nals_to_four_byte_length_prefixed(&nals)?;
            if sample.is_empty() {
                return Ok(None);
            }

            if self.session.is_none() {
                self.session = Some(AppleHevcSession::new(&self.parameter_sets)?);
            }

            self.session
                .as_mut()
                .expect("session was just initialized")
                .decode_sample(&sample)
        }
    }

    struct AppleHevcSession {
        session: VTDecompressionSessionRef,
        format_description: CMVideoFormatDescriptionRef,
        image_buffer_attrs: CFDictionaryRef,
        pixel_format_number: CFNumberRef,
    }

    // SAFETY: The session is owned by `AppleHevcSession`, released in `Drop`, and
    // all operations require `&mut self`, preventing shared concurrent use.
    unsafe impl Send for AppleHevcSession {}

    impl AppleHevcSession {
        fn new(parameter_sets: &HevcParameterSets) -> flow_like_types::Result<Self> {
            if !parameter_sets.complete() {
                bail!(
                    "H.265/HEVC stream detected, but VideoToolbox needs VPS/SPS/PPS parameter sets before decoding. \
                     Use an RTSP stream that advertises HEVC parameter sets in SDP or emits them before the first frame."
                );
            }

            let vps = parameter_sets.vps.as_ref().expect("checked complete");
            let sps = parameter_sets.sps.as_ref().expect("checked complete");
            let pps = parameter_sets.pps.as_ref().expect("checked complete");
            let parameter_set_pointers = [vps.as_ptr(), sps.as_ptr(), pps.as_ptr()];
            let parameter_set_sizes = [vps.len(), sps.len(), pps.len()];
            let mut format_description = ptr::null_mut();

            let status = unsafe {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    ptr::null(),
                    parameter_set_pointers.len(),
                    parameter_set_pointers.as_ptr(),
                    parameter_set_sizes.as_ptr(),
                    4,
                    ptr::null(),
                    &mut format_description,
                )
            };
            check_os_status(
                status,
                "CMVideoFormatDescriptionCreateFromHEVCParameterSets",
            )?;
            if format_description.is_null() {
                bail!("VideoToolbox did not create an HEVC format description");
            }

            let mut pixel_format_value = K_CV_PIXEL_FORMAT_TYPE_32_BGRA as i32;
            let pixel_format_number = unsafe {
                CFNumberCreate(
                    ptr::null(),
                    K_CF_NUMBER_SINT32_TYPE,
                    (&mut pixel_format_value as *mut i32).cast(),
                )
            };
            if pixel_format_number.is_null() {
                unsafe {
                    CFRelease(format_description.cast());
                }
                bail!("Failed to create CoreFoundation pixel format number");
            }

            let keys = [unsafe { kCVPixelBufferPixelFormatTypeKey }.cast()];
            let values = [pixel_format_number.cast()];
            let image_buffer_attrs = unsafe {
                CFDictionaryCreate(
                    ptr::null(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    keys.len() as CFIndex,
                    ptr::null(),
                    ptr::null(),
                )
            };
            if image_buffer_attrs.is_null() {
                unsafe {
                    CFRelease(pixel_format_number.cast());
                    CFRelease(format_description.cast());
                }
                bail!("Failed to create VideoToolbox image buffer attributes");
            }

            let output_callback = VTDecompressionOutputCallbackRecord {
                decompression_output_callback: Some(video_toolbox_output_callback),
                decompression_output_ref_con: ptr::null_mut(),
            };
            let mut session = ptr::null_mut();
            let status = unsafe {
                VTDecompressionSessionCreate(
                    ptr::null(),
                    format_description,
                    ptr::null(),
                    image_buffer_attrs,
                    &output_callback,
                    &mut session,
                )
            };
            if let Err(error) = check_os_status(status, "VTDecompressionSessionCreate") {
                unsafe {
                    CFRelease(image_buffer_attrs.cast());
                    CFRelease(pixel_format_number.cast());
                    CFRelease(format_description.cast());
                }
                return Err(error);
            }
            if session.is_null() {
                unsafe {
                    CFRelease(image_buffer_attrs.cast());
                    CFRelease(pixel_format_number.cast());
                    CFRelease(format_description.cast());
                }
                bail!("VideoToolbox did not create an HEVC decompression session");
            }

            Ok(Self {
                session,
                format_description,
                image_buffer_attrs,
                pixel_format_number,
            })
        }

        fn decode_sample(
            &mut self,
            sample: &[u8],
        ) -> flow_like_types::Result<Option<DynamicImage>> {
            let mut block_buffer = ptr::null_mut();
            let status = unsafe {
                CMBlockBufferCreateWithMemoryBlock(
                    ptr::null(),
                    ptr::null_mut(),
                    sample.len(),
                    ptr::null(),
                    ptr::null(),
                    0,
                    sample.len(),
                    0,
                    &mut block_buffer,
                )
            };
            check_os_status(status, "CMBlockBufferCreateWithMemoryBlock")?;
            if block_buffer.is_null() {
                bail!("CoreMedia did not create a block buffer for HEVC sample data");
            }

            let decode_result = self.decode_sample_with_block_buffer(sample, block_buffer);
            unsafe {
                CFRelease(block_buffer.cast());
            }
            decode_result
        }

        fn decode_sample_with_block_buffer(
            &mut self,
            sample: &[u8],
            block_buffer: CMBlockBufferRef,
        ) -> flow_like_types::Result<Option<DynamicImage>> {
            let status = unsafe {
                CMBlockBufferReplaceDataBytes(sample.as_ptr().cast(), block_buffer, 0, sample.len())
            };
            check_os_status(status, "CMBlockBufferReplaceDataBytes")?;

            let timing = CMSampleTimingInfo {
                duration: CMTime::INVALID,
                presentation_time_stamp: CMTime::ZERO,
                decode_time_stamp: CMTime::INVALID,
            };
            let sample_size = sample.len();
            let mut sample_buffer = ptr::null_mut();
            let status = unsafe {
                CMSampleBufferCreateReady(
                    ptr::null(),
                    block_buffer,
                    self.format_description.cast(),
                    1,
                    1,
                    &timing,
                    1,
                    &sample_size,
                    &mut sample_buffer,
                )
            };
            check_os_status(status, "CMSampleBufferCreateReady")?;
            if sample_buffer.is_null() {
                bail!("CoreMedia did not create a sample buffer for HEVC sample data");
            }

            let mut callback_state = DecodeCallbackState::default();
            let mut info_flags = 0;
            let status = unsafe {
                VTDecompressionSessionDecodeFrame(
                    self.session,
                    sample_buffer,
                    0,
                    (&mut callback_state as *mut DecodeCallbackState).cast(),
                    &mut info_flags,
                )
            };
            let wait_status =
                unsafe { VTDecompressionSessionWaitForAsynchronousFrames(self.session) };
            unsafe {
                CFRelease(sample_buffer.cast());
            }

            if status != 0 || wait_status != 0 {
                return Ok(None);
            }

            if let Some(error) = callback_state.error {
                bail!("{error}");
            }

            Ok(callback_state.image)
        }
    }

    impl Drop for AppleHevcSession {
        fn drop(&mut self) {
            unsafe {
                if !self.session.is_null() {
                    VTDecompressionSessionInvalidate(self.session);
                    CFRelease(self.session.cast());
                }
                if !self.image_buffer_attrs.is_null() {
                    CFRelease(self.image_buffer_attrs.cast());
                }
                if !self.pixel_format_number.is_null() {
                    CFRelease(self.pixel_format_number.cast());
                }
                if !self.format_description.is_null() {
                    CFRelease(self.format_description.cast());
                }
            }
        }
    }

    #[derive(Default)]
    struct DecodeCallbackState {
        image: Option<DynamicImage>,
        error: Option<String>,
    }

    extern "C" fn video_toolbox_output_callback(
        _decompression_output_ref_con: *mut c_void,
        source_frame_ref_con: *mut c_void,
        status: OSStatus,
        _info_flags: VTDecodeInfoFlags,
        image_buffer: CVPixelBufferRef,
        _presentation_time_stamp: CMTime,
        _presentation_duration: CMTime,
    ) {
        if source_frame_ref_con.is_null() {
            return;
        }

        let callback_state = unsafe { &mut *(source_frame_ref_con.cast::<DecodeCallbackState>()) };

        if status != 0 {
            callback_state.error = Some(format!(
                "VideoToolbox HEVC decode failed: OSStatus {status}"
            ));
            return;
        }

        if image_buffer.is_null() {
            return;
        }

        let result = catch_unwind(AssertUnwindSafe(|| unsafe {
            cv_pixel_buffer_to_dynamic_image(image_buffer)
        }));

        match result {
            Ok(Ok(image)) => callback_state.image = Some(image),
            Ok(Err(error)) => callback_state.error = Some(error.to_string()),
            Err(_) => {
                callback_state.error = Some("VideoToolbox HEVC callback panicked".to_string());
            }
        }
    }

    unsafe fn cv_pixel_buffer_to_dynamic_image(
        pixel_buffer: CVPixelBufferRef,
    ) -> flow_like_types::Result<DynamicImage> {
        let status =
            unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, K_CV_PIXEL_BUFFER_LOCK_READ_ONLY) };
        check_os_status(status, "CVPixelBufferLockBaseAddress")?;

        let image_result = unsafe { locked_cv_pixel_buffer_to_dynamic_image(pixel_buffer) };
        let unlock_status = unsafe {
            CVPixelBufferUnlockBaseAddress(pixel_buffer, K_CV_PIXEL_BUFFER_LOCK_READ_ONLY)
        };
        check_os_status(unlock_status, "CVPixelBufferUnlockBaseAddress")?;

        image_result
    }

    unsafe fn locked_cv_pixel_buffer_to_dynamic_image(
        pixel_buffer: CVPixelBufferRef,
    ) -> flow_like_types::Result<DynamicImage> {
        let pixel_format = unsafe { CVPixelBufferGetPixelFormatType(pixel_buffer) };
        if pixel_format != K_CV_PIXEL_FORMAT_TYPE_32_BGRA {
            bail!("VideoToolbox returned unsupported pixel format: 0x{pixel_format:08x}");
        }

        let width = unsafe { CVPixelBufferGetWidth(pixel_buffer) };
        let height = unsafe { CVPixelBufferGetHeight(pixel_buffer) };
        let bytes_per_row = unsafe { CVPixelBufferGetBytesPerRow(pixel_buffer) };
        let base_address = unsafe { CVPixelBufferGetBaseAddress(pixel_buffer) };

        if width == 0 || height == 0 || base_address.is_null() {
            bail!("VideoToolbox returned an empty HEVC pixel buffer");
        }

        let row_len = width
            .checked_mul(4)
            .ok_or_else(|| anyhow!("VideoToolbox HEVC frame width overflowed"))?;
        if bytes_per_row < row_len {
            bail!("VideoToolbox HEVC pixel buffer row stride is smaller than frame width");
        }

        let mut rgba = Vec::with_capacity(
            width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| anyhow!("VideoToolbox HEVC frame dimensions overflowed"))?,
        );
        let base = base_address.cast::<u8>();

        for y in 0..height {
            let row = unsafe { std::slice::from_raw_parts(base.add(y * bytes_per_row), row_len) };
            for bgra in row.chunks_exact(4) {
                rgba.push(bgra[2]);
                rgba.push(bgra[1]);
                rgba.push(bgra[0]);
                rgba.push(bgra[3]);
            }
        }

        let image = RgbaImage::from_raw(width as u32, height as u32, rgba)
            .ok_or_else(|| anyhow!("VideoToolbox HEVC frame had invalid RGBA dimensions"))?;
        Ok(DynamicImage::ImageRgba8(image))
    }

    fn check_os_status(status: OSStatus, operation: &str) -> flow_like_types::Result<()> {
        if status == 0 {
            Ok(())
        } else {
            bail!("{operation} failed: OSStatus {status}")
        }
    }
}

#[cfg(all(feature = "execute", target_os = "android"))]
mod platform_hevc {
    use super::*;
    use ndk::media::{
        media_codec::{
            DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec,
            MediaCodecDirection, OutputBuffer,
        },
        media_format::MediaFormat,
    };
    use std::time::Duration;

    const HEVC_MIME: &str = "video/hevc";
    const COLOR_FORMAT_YUV420_PLANAR: i32 = 19;
    const COLOR_FORMAT_YUV420_SEMIPLANAR: i32 = 21;
    const COLOR_FORMAT_YUV420_PACKED_SEMIPLANAR: i32 = 39;
    const COLOR_FORMAT_YUV420_FLEXIBLE: i32 = 0x7f42_0888;
    const BUFFER_FLAG_CODEC_CONFIG: u32 = 2;

    pub struct Decoder {
        parameter_sets: HevcParameterSets,
        codec: Option<AndroidHevcCodec>,
        frame_index: u64,
    }

    impl Decoder {
        pub fn new(parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
            Ok(Self {
                parameter_sets: HevcParameterSets::from_parameter_sets(parameter_sets),
                codec: None,
                frame_index: 0,
            })
        }

        pub fn decode(&mut self, data: &[u8]) -> flow_like_types::Result<Option<DynamicImage>> {
            let annex_b = h26x_payload_to_annex_b(data)?;
            let nals = annex_b_nals(&annex_b);
            if nals.is_empty() {
                return Ok(None);
            }

            self.parameter_sets.update_from_nals(&nals);
            if !has_decodable_hevc_nal(&nals) {
                return Ok(None);
            }

            if self.codec.is_none() {
                let dimensions = self
                    .parameter_sets
                    .dimensions()?
                    .ok_or_else(|| anyhow!("MediaCodec needs HEVC SPS dimensions before decode"))?;
                self.codec = Some(AndroidHevcCodec::new(&self.parameter_sets, dimensions)?);
            }

            let sample = annex_b.as_ref();
            let pts_us = self.frame_index.saturating_mul(33_333);
            self.frame_index = self.frame_index.saturating_add(1);
            self.codec
                .as_mut()
                .expect("codec was just initialized")
                .decode_sample(sample, pts_us)
        }
    }

    struct AndroidHevcCodec {
        codec: MediaCodec,
        output_format: MediaFormat,
        dimensions: HevcDimensions,
    }

    impl AndroidHevcCodec {
        fn new(
            parameter_sets: &HevcParameterSets,
            dimensions: HevcDimensions,
        ) -> flow_like_types::Result<Self> {
            if !parameter_sets.complete() {
                bail!(
                    "H.265/HEVC stream detected, but MediaCodec needs VPS/SPS/PPS parameter sets before decoding. \
                     Use an RTSP stream that advertises HEVC parameter sets in SDP or emits them before the first frame."
                );
            }

            let mut format = MediaFormat::new();
            format.set_str("mime", HEVC_MIME);
            format.set_i32("width", checked_i32(dimensions.coded_width, "HEVC width")?);
            format.set_i32(
                "height",
                checked_i32(dimensions.coded_height, "HEVC height")?,
            );
            format.set_buffer("csd-0", &parameter_sets.to_annex_b());
            format.set_i32("color-format", COLOR_FORMAT_YUV420_FLEXIBLE);

            let codec = MediaCodec::from_decoder_type(HEVC_MIME)
                .ok_or_else(|| anyhow!("Android MediaCodec did not provide an HEVC decoder"))?;
            codec
                .configure(&format, None, MediaCodecDirection::Decoder)
                .map_err(|e| anyhow!("Failed to configure Android HEVC MediaCodec: {e}"))?;
            codec
                .start()
                .map_err(|e| anyhow!("Failed to start Android HEVC MediaCodec: {e}"))?;
            let output_format = codec.output_format();

            Ok(Self {
                codec,
                output_format,
                dimensions,
            })
        }

        fn decode_sample(
            &mut self,
            sample: &[u8],
            pts_us: u64,
        ) -> flow_like_types::Result<Option<DynamicImage>> {
            self.queue_input(sample, pts_us)?;
            self.drain_output()
        }

        fn queue_input(&self, sample: &[u8], pts_us: u64) -> flow_like_types::Result<()> {
            let mut input = match self
                .codec
                .dequeue_input_buffer(Duration::from_millis(20))
                .map_err(|e| anyhow!("Android HEVC MediaCodec input dequeue failed: {e}"))?
            {
                DequeuedInputBufferResult::Buffer(input) => input,
                DequeuedInputBufferResult::TryAgainLater => return Ok(()),
            };

            {
                let buffer = input.buffer_mut();
                if sample.len() > buffer.len() {
                    bail!(
                        "Android HEVC MediaCodec input buffer too small: {} < {}",
                        buffer.len(),
                        sample.len()
                    );
                }

                for (dst, src) in buffer.iter_mut().zip(sample.iter()) {
                    dst.write(*src);
                }
            }

            self.codec
                .queue_input_buffer(input, 0, sample.len(), pts_us, 0)
                .map_err(|e| anyhow!("Android HEVC MediaCodec input queue failed: {e}"))
        }

        fn drain_output(&mut self) -> flow_like_types::Result<Option<DynamicImage>> {
            for _ in 0..8 {
                match self
                    .codec
                    .dequeue_output_buffer(Duration::from_millis(20))
                    .map_err(|e| anyhow!("Android HEVC MediaCodec output dequeue failed: {e}"))?
                {
                    DequeuedOutputBufferInfoResult::Buffer(output) => {
                        if output.info().flags() & BUFFER_FLAG_CODEC_CONFIG != 0
                            || output.info().size() <= 0
                        {
                            self.codec
                                .release_output_buffer(output, false)
                                .map_err(|e| {
                                    anyhow!("Android HEVC MediaCodec output release failed: {e}")
                                })?;
                            continue;
                        }

                        let image = android_output_buffer_to_image(
                            &output,
                            &self.output_format,
                            self.dimensions,
                        );
                        self.codec
                            .release_output_buffer(output, false)
                            .map_err(|e| {
                                anyhow!("Android HEVC MediaCodec output release failed: {e}")
                            })?;
                        return image.map(Some);
                    }
                    DequeuedOutputBufferInfoResult::OutputFormatChanged => {
                        self.output_format = self.codec.output_format();
                    }
                    DequeuedOutputBufferInfoResult::OutputBuffersChanged => {}
                    DequeuedOutputBufferInfoResult::TryAgainLater => return Ok(None),
                }
            }

            Ok(None)
        }
    }

    impl Drop for AndroidHevcCodec {
        fn drop(&mut self) {
            let _ = self.codec.stop();
        }
    }

    fn android_output_buffer_to_image(
        output: &OutputBuffer<'_>,
        format: &MediaFormat,
        fallback_dimensions: HevcDimensions,
    ) -> flow_like_types::Result<DynamicImage> {
        let size = usize::try_from(output.info().size())
            .map_err(|_| anyhow!("Android HEVC MediaCodec returned negative output size"))?;
        let buffer = output.buffer();
        if size > buffer.len() {
            bail!("Android HEVC MediaCodec output buffer size exceeds buffer length");
        }

        // Android's NDK docs mark AMediaCodecBufferInfo.offset invalid before/at API 35.
        // The returned output buffer already points at the image payload; size remains valid.
        let payload = &buffer[..size];
        let layout = AndroidYuvLayout::from_format(format, fallback_dimensions)?;
        android_yuv420_to_image(payload, layout)
    }

    #[derive(Clone, Copy)]
    struct AndroidYuvLayout {
        crop_left: usize,
        crop_top: usize,
        crop_width: usize,
        crop_height: usize,
        stride: usize,
        slice_height: usize,
        color_format: i32,
    }

    impl AndroidYuvLayout {
        fn from_format(
            format: &MediaFormat,
            fallback_dimensions: HevcDimensions,
        ) -> flow_like_types::Result<Self> {
            let coded_width = format
                .i32("width")
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .unwrap_or(fallback_dimensions.coded_width as usize);
            let coded_height = format
                .i32("height")
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .unwrap_or(fallback_dimensions.coded_height as usize);
            let stride = format
                .i32("stride")
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .unwrap_or(coded_width);
            let slice_height = format
                .i32("slice-height")
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .unwrap_or(coded_height);
            let color_format = format
                .i32("color-format")
                .unwrap_or(COLOR_FORMAT_YUV420_FLEXIBLE);

            let crop_left = format
                .i32("crop-left")
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0);
            let crop_top = format
                .i32("crop-top")
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0);
            let crop_right = format
                .i32("crop-right")
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or_else(|| fallback_dimensions.display_width.saturating_sub(1) as usize);
            let crop_bottom = format
                .i32("crop-bottom")
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or_else(|| fallback_dimensions.display_height.saturating_sub(1) as usize);

            if crop_right < crop_left || crop_bottom < crop_top {
                bail!("Android HEVC MediaCodec output crop rectangle is invalid");
            }

            let crop_width = crop_right - crop_left + 1;
            let crop_height = crop_bottom - crop_top + 1;
            if crop_left + crop_width > coded_width || crop_top + crop_height > coded_height {
                bail!("Android HEVC MediaCodec output crop exceeds coded dimensions");
            }

            Ok(Self {
                crop_left,
                crop_top,
                crop_width,
                crop_height,
                stride,
                slice_height,
                color_format,
            })
        }
    }

    fn android_yuv420_to_image(
        payload: &[u8],
        layout: AndroidYuvLayout,
    ) -> flow_like_types::Result<DynamicImage> {
        match layout.color_format {
            COLOR_FORMAT_YUV420_PLANAR => android_planar_yuv420_to_image(payload, layout),
            COLOR_FORMAT_YUV420_SEMIPLANAR | COLOR_FORMAT_YUV420_PACKED_SEMIPLANAR => {
                android_semiplanar_yuv420_to_image(payload, layout)
            }
            COLOR_FORMAT_YUV420_FLEXIBLE => android_semiplanar_yuv420_to_image(payload, layout)
                .or_else(|_| android_planar_yuv420_to_image(payload, layout)),
            color_format => bail!(
                "Android HEVC MediaCodec returned unsupported YUV color format: {color_format}"
            ),
        }
    }

    fn android_planar_yuv420_to_image(
        payload: &[u8],
        layout: AndroidYuvLayout,
    ) -> flow_like_types::Result<DynamicImage> {
        let y_plane_len = layout
            .stride
            .checked_mul(layout.slice_height)
            .ok_or_else(|| anyhow!("Android HEVC Y plane size overflowed"))?;
        let chroma_stride = layout.stride.div_ceil(2);
        let chroma_height = layout.slice_height.div_ceil(2);
        let chroma_plane_len = chroma_stride
            .checked_mul(chroma_height)
            .ok_or_else(|| anyhow!("Android HEVC chroma plane size overflowed"))?;
        let u_offset = y_plane_len;
        let v_offset = u_offset
            .checked_add(chroma_plane_len)
            .ok_or_else(|| anyhow!("Android HEVC V plane offset overflowed"))?;
        let required_len = v_offset
            .checked_add(chroma_plane_len)
            .ok_or_else(|| anyhow!("Android HEVC planar buffer size overflowed"))?;
        if payload.len() < required_len {
            bail!("Android HEVC planar YUV buffer is shorter than expected");
        }

        yuv420_to_rgb_image(layout, |x, y| {
            let source_x = layout.crop_left + x;
            let source_y = layout.crop_top + y;
            let y_value = payload[source_y * layout.stride + source_x];
            let chroma_x = source_x / 2;
            let chroma_y = source_y / 2;
            let u = payload[u_offset + chroma_y * chroma_stride + chroma_x];
            let v = payload[v_offset + chroma_y * chroma_stride + chroma_x];
            (y_value, u, v)
        })
    }

    fn android_semiplanar_yuv420_to_image(
        payload: &[u8],
        layout: AndroidYuvLayout,
    ) -> flow_like_types::Result<DynamicImage> {
        let y_plane_len = layout
            .stride
            .checked_mul(layout.slice_height)
            .ok_or_else(|| anyhow!("Android HEVC Y plane size overflowed"))?;
        let chroma_height = layout.slice_height.div_ceil(2);
        let uv_plane_len = layout
            .stride
            .checked_mul(chroma_height)
            .ok_or_else(|| anyhow!("Android HEVC UV plane size overflowed"))?;
        let required_len = y_plane_len
            .checked_add(uv_plane_len)
            .ok_or_else(|| anyhow!("Android HEVC semiplanar buffer size overflowed"))?;
        if payload.len() < required_len {
            bail!("Android HEVC semiplanar YUV buffer is shorter than expected");
        }

        yuv420_to_rgb_image(layout, |x, y| {
            let source_x = layout.crop_left + x;
            let source_y = layout.crop_top + y;
            let y_value = payload[source_y * layout.stride + source_x];
            let uv_index = y_plane_len + (source_y / 2) * layout.stride + (source_x / 2) * 2;
            let u = payload[uv_index];
            let v = payload[uv_index + 1];
            (y_value, u, v)
        })
    }

    fn yuv420_to_rgb_image(
        layout: AndroidYuvLayout,
        mut pixel: impl FnMut(usize, usize) -> (u8, u8, u8),
    ) -> flow_like_types::Result<DynamicImage> {
        let mut rgb = Vec::with_capacity(
            layout
                .crop_width
                .checked_mul(layout.crop_height)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or_else(|| anyhow!("Android HEVC RGB buffer size overflowed"))?,
        );

        for y in 0..layout.crop_height {
            for x in 0..layout.crop_width {
                let (y_value, u, v) = pixel(x, y);
                rgb.extend_from_slice(&yuv_to_rgb(y_value, u, v));
            }
        }

        let image = RgbImage::from_raw(layout.crop_width as u32, layout.crop_height as u32, rgb)
            .ok_or_else(|| anyhow!("Android HEVC frame had invalid RGB dimensions"))?;
        Ok(DynamicImage::ImageRgb8(image))
    }

    fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
        let c = i32::from(y).saturating_sub(16);
        let d = i32::from(u).saturating_sub(128);
        let e = i32::from(v).saturating_sub(128);
        [
            clamp_u8((298 * c + 409 * e + 128) >> 8),
            clamp_u8((298 * c - 100 * d - 208 * e + 128) >> 8),
            clamp_u8((298 * c + 516 * d + 128) >> 8),
        ]
    }

    fn clamp_u8(value: i32) -> u8 {
        value.clamp(0, 255) as u8
    }

    fn checked_i32(value: u32, name: &str) -> flow_like_types::Result<i32> {
        i32::try_from(value).map_err(|_| anyhow!("{name} is too large for MediaCodec"))
    }
}

#[cfg(all(feature = "execute", target_os = "windows"))]
mod platform_hevc {
    use super::*;
    use std::{mem::ManuallyDrop, ptr, sync::mpsc, thread};
    use windows::{
        Win32::{
            Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK},
            Media::MediaFoundation::{
                IMFActivate, IMFMediaType, IMFSample, IMFTransform, MF_E_TRANSFORM_NEED_MORE_INPUT,
                MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_DEFAULT_STRIDE, MF_MT_FIXED_SIZE_SAMPLES,
                MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SAMPLE_SIZE, MF_MT_SUBTYPE, MF_VERSION,
                MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
                MFSTARTUP_FULL, MFShutdown, MFStartup, MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG_ALL, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
                MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_REGISTER_TYPE_INFO,
                MFTEnumEx, MFVideoFormat_HEVC_ES, MFVideoFormat_NV12, MFVideoFormat_P010,
            },
            System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        },
        core::GUID,
    };

    pub struct Decoder {
        requests: Option<mpsc::Sender<DecodeRequest>>,
        worker: Option<thread::JoinHandle<()>>,
    }

    struct DecodeRequest {
        data: Vec<u8>,
        response: mpsc::Sender<flow_like_types::Result<Option<DynamicImage>>>,
    }

    impl Decoder {
        pub fn new(parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
            let (requests, request_rx) = mpsc::channel::<DecodeRequest>();
            let worker = thread::Builder::new()
                .name("flow-like-hevc-decoder".to_string())
                .spawn(move || {
                    let mut decoder = WindowsHevcDecoderCore::new(parameter_sets);
                    while let Ok(request) = request_rx.recv() {
                        let result = decoder.decode(&request.data);
                        let _ = request.response.send(result);
                    }
                })
                .map_err(|e| anyhow!("Failed to start Windows HEVC decoder worker: {e}"))?;

            Ok(Self {
                requests: Some(requests),
                worker: Some(worker),
            })
        }

        pub fn decode(&mut self, data: &[u8]) -> flow_like_types::Result<Option<DynamicImage>> {
            let requests = self
                .requests
                .as_ref()
                .ok_or_else(|| anyhow!("Windows HEVC decoder worker has stopped"))?;
            let (response, response_rx) = mpsc::channel();
            requests
                .send(DecodeRequest {
                    data: data.to_vec(),
                    response,
                })
                .map_err(|_| anyhow!("Windows HEVC decoder worker has stopped"))?;

            response_rx
                .recv()
                .map_err(|_| anyhow!("Windows HEVC decoder worker stopped before decoding"))?
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            self.requests.take();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    struct WindowsHevcDecoderCore {
        parameter_sets: HevcParameterSets,
        session: Option<WindowsHevcSession>,
        frame_index: u64,
    }

    impl WindowsHevcDecoderCore {
        fn new(parameter_sets: Vec<Vec<u8>>) -> Self {
            Self {
                parameter_sets: HevcParameterSets::from_parameter_sets(parameter_sets),
                session: None,
                frame_index: 0,
            }
        }

        fn decode(&mut self, data: &[u8]) -> flow_like_types::Result<Option<DynamicImage>> {
            let annex_b = h26x_payload_to_annex_b(data)?;
            let nals = annex_b_nals(&annex_b);
            if nals.is_empty() {
                return Ok(None);
            }

            self.parameter_sets.update_from_nals(&nals);
            if !has_decodable_hevc_nal(&nals) {
                return Ok(None);
            }

            if self.session.is_none() {
                let dimensions = self.parameter_sets.dimensions()?.ok_or_else(|| {
                    anyhow!("Media Foundation needs HEVC SPS dimensions before decode")
                })?;
                self.session = Some(WindowsHevcSession::new(dimensions)?);
            }

            let mut sample = self.parameter_sets.to_annex_b();
            sample.extend_from_slice(annex_b.as_ref());
            let sample_time = self
                .frame_index
                .saturating_mul(333_333)
                .try_into()
                .unwrap_or(i64::MAX);
            self.frame_index = self.frame_index.saturating_add(1);

            self.session
                .as_mut()
                .expect("session was just initialized")
                .decode_sample(&sample, sample_time)
        }
    }

    struct WindowsHevcSession {
        _guard: MediaFoundationGuard,
        transform: IMFTransform,
        input_stream_id: u32,
        output_stream_id: u32,
        dimensions: HevcDimensions,
        output_format: WindowsHevcOutputFormat,
        output_buffer_len: u32,
    }

    impl WindowsHevcSession {
        fn new(dimensions: HevcDimensions) -> flow_like_types::Result<Self> {
            let guard = MediaFoundationGuard::new()?;
            let transform = create_hevc_decoder_transform()?;
            let input_stream_id = 0;
            let output_stream_id = 0;
            let width = dimensions.display_width;
            let height = dimensions.display_height;

            unsafe {
                let input_type =
                    create_video_media_type(MFVideoFormat_HEVC_ES, width, height, None, None)?;
                transform
                    .SetInputType(input_stream_id, &input_type, 0)
                    .map_err(|e| anyhow!("Media Foundation HEVC SetInputType failed: {e}"))?;

                let (output_format, output_buffer_len) =
                    set_windows_hevc_output_type(&transform, output_stream_id, width, height)?;
                transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                    .map_err(|e| {
                        anyhow!("Media Foundation HEVC begin streaming message failed: {e}")
                    })?;
                transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                    .map_err(|e| {
                        anyhow!("Media Foundation HEVC start stream message failed: {e}")
                    })?;

                Ok(Self {
                    _guard: guard,
                    transform,
                    input_stream_id,
                    output_stream_id,
                    dimensions,
                    output_format,
                    output_buffer_len,
                })
            }
        }

        fn decode_sample(
            &mut self,
            sample: &[u8],
            sample_time: i64,
        ) -> flow_like_types::Result<Option<DynamicImage>> {
            let input_sample = unsafe { create_input_sample(sample, sample_time)? };
            unsafe {
                self.transform
                    .ProcessInput(self.input_stream_id, &input_sample, 0)
                    .map_err(|e| anyhow!("Media Foundation HEVC ProcessInput failed: {e}"))?;
            }

            self.process_output()
        }

        fn process_output(&mut self) -> flow_like_types::Result<Option<DynamicImage>> {
            let output_sample = unsafe { MFCreateSample() }
                .map_err(|e| anyhow!("Media Foundation MFCreateSample failed: {e}"))?;
            let output_buffer = unsafe { MFCreateMemoryBuffer(self.output_buffer_len) }
                .map_err(|e| anyhow!("Media Foundation MFCreateMemoryBuffer failed: {e}"))?;
            unsafe {
                output_sample
                    .AddBuffer(&output_buffer)
                    .map_err(|e| anyhow!("Media Foundation output AddBuffer failed: {e}"))?;
            }

            let mut data_buffer = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: self.output_stream_id,
                pSample: ManuallyDrop::new(Some(output_sample.clone())),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0u32;
            let output_result = unsafe {
                self.transform
                    .ProcessOutput(0, std::slice::from_mut(&mut data_buffer), &mut status)
            };

            match output_result {
                Ok(()) => unsafe {
                    windows_hevc_sample_to_image(
                        &output_sample,
                        self.dimensions,
                        self.output_format,
                    )
                    .map(Some)
                },
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
                Err(error) => Err(anyhow!(
                    "Media Foundation HEVC ProcessOutput failed: {error}"
                )),
            }
        }
    }

    struct MediaFoundationGuard {
        co_initialized: bool,
    }

    impl MediaFoundationGuard {
        fn new() -> flow_like_types::Result<Self> {
            let co_init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if co_init != S_OK && co_init != S_FALSE && co_init != RPC_E_CHANGED_MODE {
                return Err(anyhow!("Windows COM initialization failed: {co_init:?}"));
            }

            unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
                .map_err(|e| anyhow!("Media Foundation startup failed: {e}"))?;

            Ok(Self {
                co_initialized: co_init == S_OK || co_init == S_FALSE,
            })
        }
    }

    impl Drop for MediaFoundationGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                if self.co_initialized {
                    CoUninitialize();
                }
            }
        }
    }

    fn create_hevc_decoder_transform() -> flow_like_types::Result<IMFTransform> {
        let input_type = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_HEVC_ES,
        };
        let mut activates: *mut Option<IMFActivate> = ptr::null_mut();
        let mut count = 0u32;

        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG_ALL,
                Some(&input_type),
                None,
                &mut activates,
                &mut count,
            )
        }
        .map_err(|e| anyhow!("Media Foundation HEVC decoder enumeration failed: {e}"))?;

        if activates.is_null() || count == 0 {
            bail!(
                "Windows Media Foundation did not find an HEVC decoder. Install/enable the platform HEVC video extension or use an H.264 camera profile."
            );
        }

        let result = unsafe { activate_first_transform(activates, count) };
        unsafe {
            CoTaskMemFree(Some(activates.cast()));
        }
        result
    }

    unsafe fn activate_first_transform(
        activates: *mut Option<IMFActivate>,
        count: u32,
    ) -> flow_like_types::Result<IMFTransform> {
        let activate_slice = unsafe { std::slice::from_raw_parts_mut(activates, count as usize) };
        let mut last_error = None;

        for activate_slot in activate_slice {
            let Some(activate) = activate_slot.take() else {
                continue;
            };

            match unsafe { activate.ActivateObject::<IMFTransform>() } {
                Ok(transform) => {
                    let _ = unsafe { activate.ShutdownObject() };
                    return Ok(transform);
                }
                Err(error) => {
                    last_error = Some(error.to_string());
                    let _ = unsafe { activate.ShutdownObject() };
                }
            }
        }

        bail!(
            "Media Foundation found HEVC decoder transforms but could not activate one{}",
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )
    }

    unsafe fn create_video_media_type(
        subtype: GUID,
        width: u32,
        height: u32,
        sample_size: Option<u32>,
        default_stride: Option<u32>,
    ) -> flow_like_types::Result<IMFMediaType> {
        let media_type = unsafe { MFCreateMediaType() }
            .map_err(|e| anyhow!("Media Foundation MFCreateMediaType failed: {e}"))?;
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|e| anyhow!("Media Foundation media type major failed: {e}"))?;
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &subtype)
                .map_err(|e| anyhow!("Media Foundation media type subtype failed: {e}"))?;
            media_type
                .SetUINT64(
                    &MF_MT_FRAME_SIZE,
                    (u64::from(width) << 32) | u64::from(height),
                )
                .map_err(|e| anyhow!("Media Foundation media type frame size failed: {e}"))?;
            if let Some(default_stride) = default_stride {
                media_type
                    .SetUINT32(&MF_MT_DEFAULT_STRIDE, default_stride)
                    .map_err(|e| anyhow!("Media Foundation default stride failed: {e}"))?;
            }
            if let Some(sample_size) = sample_size {
                media_type
                    .SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)
                    .map_err(|e| anyhow!("Media Foundation fixed sample flag failed: {e}"))?;
                media_type
                    .SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)
                    .map_err(|e| anyhow!("Media Foundation independent sample flag failed: {e}"))?;
                media_type
                    .SetUINT32(&MF_MT_SAMPLE_SIZE, sample_size)
                    .map_err(|e| anyhow!("Media Foundation sample size failed: {e}"))?;
            }
        }
        Ok(media_type)
    }

    unsafe fn create_input_sample(
        data: &[u8],
        sample_time: i64,
    ) -> flow_like_types::Result<IMFSample> {
        let buffer_len = checked_u32(data.len(), "HEVC input sample size")?;
        let sample = unsafe { MFCreateSample() }
            .map_err(|e| anyhow!("Media Foundation MFCreateSample failed: {e}"))?;
        let buffer = unsafe { MFCreateMemoryBuffer(buffer_len) }
            .map_err(|e| anyhow!("Media Foundation MFCreateMemoryBuffer failed: {e}"))?;

        let mut dst = ptr::null_mut();
        unsafe {
            buffer
                .Lock(&mut dst, None, None)
                .map_err(|e| anyhow!("Media Foundation input buffer lock failed: {e}"))?;
            ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
            buffer
                .Unlock()
                .map_err(|e| anyhow!("Media Foundation input buffer unlock failed: {e}"))?;
            buffer
                .SetCurrentLength(buffer_len)
                .map_err(|e| anyhow!("Media Foundation input buffer length failed: {e}"))?;
            sample
                .AddBuffer(&buffer)
                .map_err(|e| anyhow!("Media Foundation input AddBuffer failed: {e}"))?;
            sample
                .SetSampleTime(sample_time)
                .map_err(|e| anyhow!("Media Foundation input sample time failed: {e}"))?;
        }

        Ok(sample)
    }

    #[derive(Clone, Copy)]
    enum WindowsHevcOutputFormat {
        Nv12,
        P010,
    }

    impl WindowsHevcOutputFormat {
        fn subtype(self) -> GUID {
            match self {
                Self::Nv12 => MFVideoFormat_NV12,
                Self::P010 => MFVideoFormat_P010,
            }
        }

        fn name(self) -> &'static str {
            match self {
                Self::Nv12 => "NV12",
                Self::P010 => "P010",
            }
        }

        fn sample_size(self, width: u32, height: u32) -> flow_like_types::Result<u32> {
            let pixels = width
                .checked_mul(height)
                .ok_or_else(|| anyhow!("Media Foundation HEVC output size overflowed"))?;
            let size = match self {
                Self::Nv12 => pixels
                    .checked_mul(3)
                    .and_then(|value| value.checked_div(2))
                    .ok_or_else(|| anyhow!("Media Foundation NV12 output size overflowed"))?,
                Self::P010 => pixels
                    .checked_mul(3)
                    .ok_or_else(|| anyhow!("Media Foundation P010 output size overflowed"))?,
            };
            Ok(size)
        }
    }

    fn set_windows_hevc_output_type(
        transform: &IMFTransform,
        output_stream_id: u32,
        width: u32,
        height: u32,
    ) -> flow_like_types::Result<(WindowsHevcOutputFormat, u32)> {
        let mut last_error = None;

        for output_format in [WindowsHevcOutputFormat::Nv12, WindowsHevcOutputFormat::P010] {
            let sample_size = output_format.sample_size(width, height)?;
            let output_type = unsafe {
                create_video_media_type(
                    output_format.subtype(),
                    width,
                    height,
                    Some(sample_size),
                    Some(width),
                )?
            };

            let set_type_result =
                unsafe { transform.SetOutputType(output_stream_id, &output_type, 0) };
            if let Err(error) = set_type_result {
                last_error = Some(format!(
                    "{} output was rejected: {error}",
                    output_format.name()
                ));
                continue;
            }

            let output_info = unsafe { transform.GetOutputStreamInfo(output_stream_id) }
                .map_err(|e| anyhow!("Media Foundation HEVC GetOutputStreamInfo failed: {e}"))?;
            return Ok((output_format, output_info.cbSize.max(sample_size)));
        }

        bail!(
            "Media Foundation HEVC decoder did not accept NV12 or P010 output{}",
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )
    }

    unsafe fn windows_hevc_sample_to_image(
        sample: &IMFSample,
        dimensions: HevcDimensions,
        output_format: WindowsHevcOutputFormat,
    ) -> flow_like_types::Result<DynamicImage> {
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }
            .map_err(|e| anyhow!("Media Foundation output contiguous buffer failed: {e}"))?;
        let current_len = unsafe { buffer.GetCurrentLength() }
            .map_err(|e| anyhow!("Media Foundation output buffer length failed: {e}"))?
            as usize;
        let width = dimensions.display_width as usize;
        let height = dimensions.display_height as usize;
        let required_len = usize::try_from(
            output_format.sample_size(dimensions.display_width, dimensions.display_height)?,
        )
        .map_err(|_| anyhow!("Media Foundation HEVC output size is too large"))?;
        if current_len < required_len {
            bail!("Media Foundation HEVC RGB output is shorter than expected");
        }

        let mut ptr = ptr::null_mut();
        unsafe {
            buffer
                .Lock(&mut ptr, None, None)
                .map_err(|e| anyhow!("Media Foundation output buffer lock failed: {e}"))?;
        }
        let image_result = unsafe {
            let bytes = std::slice::from_raw_parts(ptr, required_len);
            windows_hevc_bytes_to_image(bytes, width, height, output_format)
        };
        unsafe {
            buffer
                .Unlock()
                .map_err(|e| anyhow!("Media Foundation output buffer unlock failed: {e}"))?;
        }

        image_result
    }

    fn windows_hevc_bytes_to_image(
        bytes: &[u8],
        width: usize,
        height: usize,
        output_format: WindowsHevcOutputFormat,
    ) -> flow_like_types::Result<DynamicImage> {
        match output_format {
            WindowsHevcOutputFormat::Nv12 => nv12_bytes_to_image(bytes, width, height),
            WindowsHevcOutputFormat::P010 => p010_bytes_to_image(bytes, width, height),
        }
    }

    fn nv12_bytes_to_image(
        bytes: &[u8],
        width: usize,
        height: usize,
    ) -> flow_like_types::Result<DynamicImage> {
        let y_plane_len = width
            .checked_mul(height)
            .ok_or_else(|| anyhow!("Media Foundation NV12 luma size overflowed"))?;
        let uv_plane_len = y_plane_len / 2;
        let required_len = y_plane_len
            .checked_add(uv_plane_len)
            .ok_or_else(|| anyhow!("Media Foundation NV12 frame size overflowed"))?;
        if bytes.len() < required_len {
            bail!("Media Foundation NV12 output is shorter than expected");
        }

        let mut rgb = Vec::with_capacity(
            width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or_else(|| anyhow!("Media Foundation RGB image size overflowed"))?,
        );

        for y in 0..height {
            for x in 0..width {
                let y_value = bytes[y * width + x];
                let uv_index = y_plane_len + (y / 2) * width + (x / 2) * 2;
                let u = bytes[uv_index];
                let v = bytes[uv_index + 1];
                rgb.extend_from_slice(&windows_yuv_to_rgb(y_value, u, v));
            }
        }

        let image = RgbImage::from_raw(width as u32, height as u32, rgb)
            .ok_or_else(|| anyhow!("Media Foundation HEVC frame had invalid RGB dimensions"))?;
        Ok(DynamicImage::ImageRgb8(image))
    }

    fn p010_bytes_to_image(
        bytes: &[u8],
        width: usize,
        height: usize,
    ) -> flow_like_types::Result<DynamicImage> {
        let y_plane_len = width
            .checked_mul(height)
            .and_then(|samples| samples.checked_mul(2))
            .ok_or_else(|| anyhow!("Media Foundation P010 luma size overflowed"))?;
        let uv_plane_len = width
            .checked_mul(height)
            .ok_or_else(|| anyhow!("Media Foundation P010 chroma size overflowed"))?;
        let required_len = y_plane_len
            .checked_add(uv_plane_len)
            .ok_or_else(|| anyhow!("Media Foundation P010 frame size overflowed"))?;
        if bytes.len() < required_len {
            bail!("Media Foundation P010 output is shorter than expected");
        }

        let mut rgb = Vec::with_capacity(
            width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or_else(|| anyhow!("Media Foundation RGB image size overflowed"))?,
        );

        for y in 0..height {
            for x in 0..width {
                let y_value = p010_sample_to_u8(bytes, (y * width + x) * 2)?;
                let uv_index = y_plane_len + ((y / 2) * width + (x / 2) * 2) * 2;
                let u = p010_sample_to_u8(bytes, uv_index)?;
                let v = p010_sample_to_u8(bytes, uv_index + 2)?;
                rgb.extend_from_slice(&windows_yuv_to_rgb(y_value, u, v));
            }
        }

        let image = RgbImage::from_raw(width as u32, height as u32, rgb)
            .ok_or_else(|| anyhow!("Media Foundation HEVC frame had invalid RGB dimensions"))?;
        Ok(DynamicImage::ImageRgb8(image))
    }

    fn p010_sample_to_u8(bytes: &[u8], offset: usize) -> flow_like_types::Result<u8> {
        let sample = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| anyhow!("Media Foundation P010 sample exceeded output buffer"))?;
        Ok((u16::from_le_bytes([sample[0], sample[1]]) >> 8) as u8)
    }

    fn windows_yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
        let c = i32::from(y).saturating_sub(16);
        let d = i32::from(u).saturating_sub(128);
        let e = i32::from(v).saturating_sub(128);
        [
            clamp_u8((298 * c + 409 * e + 128) >> 8),
            clamp_u8((298 * c - 100 * d - 208 * e + 128) >> 8),
            clamp_u8((298 * c + 516 * d + 128) >> 8),
        ]
    }

    fn clamp_u8(value: i32) -> u8 {
        value.clamp(0, 255) as u8
    }

    fn checked_u32(value: usize, name: &str) -> flow_like_types::Result<u32> {
        u32::try_from(value).map_err(|_| anyhow!("{name} is too large for Media Foundation"))
    }
}

#[cfg(all(
    feature = "execute",
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "android",
        target_os = "windows"
    ))
))]
mod platform_hevc {
    use super::*;

    pub struct Decoder {}

    impl Decoder {
        pub fn new(_parameter_sets: Vec<Vec<u8>>) -> flow_like_types::Result<Self> {
            bail!(
                "H.265/HEVC stream detected, but bundled software HEVC decoding is disabled. \
                 This build must use the platform decoder backend for {} ({}), and that backend is not implemented for this target yet. \
                 Use an H.264 camera profile/substream or a JPEG/MJPEG snapshot URL.",
                platform_target_name(),
                platform_hevc_backend_name()
            )
        }

        pub fn decode(&mut self, _data: &[u8]) -> flow_like_types::Result<Option<DynamicImage>> {
            Ok(None)
        }
    }
}

#[cfg(feature = "execute")]
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

#[cfg(feature = "execute")]
fn starts_with_annex_b_start_code(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1])
}

#[cfg(feature = "execute")]
fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let Some((first_start, first_start_len)) = find_annex_b_start_code(data, 0) else {
        return (!data.is_empty()).then_some(data).into_iter().collect();
    };

    let mut nals = Vec::new();
    let mut nal_start = first_start + first_start_len;

    while nal_start < data.len() {
        let next_start = find_annex_b_start_code(data, nal_start).map(|(start, _)| start);
        let nal_end = next_start.unwrap_or(data.len());

        if nal_end > nal_start {
            nals.push(&data[nal_start..nal_end]);
        }

        let Some((start, start_len)) = next_start.and_then(|start| {
            find_annex_b_start_code(data, start).map(|(_, start_len)| (start, start_len))
        }) else {
            break;
        };

        nal_start = start + start_len;
    }

    nals
}

#[cfg(feature = "execute")]
fn find_annex_b_start_code(data: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut offset = from;
    while offset + 3 <= data.len() {
        if data[offset..].starts_with(&[0, 0, 1]) {
            return Some((offset, 3));
        }

        if offset + 4 <= data.len() && data[offset..].starts_with(&[0, 0, 0, 1]) {
            return Some((offset, 4));
        }

        offset += 1;
    }

    None
}

#[cfg(feature = "execute")]
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

#[cfg(feature = "execute")]
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

#[cfg(all(test, feature = "execute"))]
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
