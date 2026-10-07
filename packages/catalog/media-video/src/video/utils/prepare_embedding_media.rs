use super::*;
use flow_like_catalog_core::EmbeddingContent;
#[cfg(feature = "execute")]
use flow_like_catalog_core::{
    EmbeddingAudio, EmbeddingContentPart, EmbeddingVideo, EmbeddingVideoFrame, NodeImage,
};
#[cfg(feature = "execute")]
use flow_like_model_provider::embedding::interface::{
    EmbeddingInput, EmbeddingPart, VideoFrame, VideoInput,
};

#[crate::register_node]
#[derive(Default)]
pub struct PrepareEmbeddingMediaNode;

impl PrepareEmbeddingMediaNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PrepareEmbeddingMediaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "prepare_embedding_media",
            "Prepare Embedding Media",
            "Decodes an audio or video file into content accepted by Embed Content",
            "AI/Embedding",
        );
        node.set_flowscript_name("ai.embedding", "prepareMedia");
        node.set_version(1);
        node.set_long_running(true);
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Audio or video file");
        node.add_input_pin(
            "kind",
            "Kind",
            "Which media input to prepare",
            VariableType::String,
        )
        .set_default_value(Some(json!("audio")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["audio".into(), "video".into()])
                .build(),
        );
        node.add_input_pin(
            "max_frames",
            "Max Frames",
            "Maximum uniformly sampled video frames",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(16)));
        node.add_input_pin(
            "include_audio",
            "Include Audio",
            "Include the video's audio track; missing audio returns an error",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_output_pin(
            "content",
            "Content",
            "Decoded media for Embed Content",
            VariableType::Struct,
        )
        .set_schema::<EmbeddingContent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let kind: String = context.evaluate_pin("kind").await?;
        let kind = match kind.as_str() {
            "audio" => EmbeddingMediaKind::Audio,
            "video" => {
                let max_frames: i64 = context.evaluate_pin("max_frames").await?;
                let max_frames = usize::try_from(max_frames).map_err(|_| {
                    flow_like_types::anyhow!("Max Frames must be between 1 and 1024")
                })?;
                let include_audio: bool = context.evaluate_pin("include_audio").await?;
                EmbeddingMediaKind::Video {
                    max_frames,
                    include_audio,
                }
            }
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Media kind must be audio or video"
                ));
            }
        };
        let (store, location) = flow_path_object(context, &source).await?;
        let content = decode_embedding_media(store.as_ref(), &location, kind)
            .await?
            .into_content(context)
            .await?;
        context.set_pin_value("content", json!(content)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EmbeddingMediaKind {
    Text,
    Image,
    Audio,
    Video {
        max_frames: usize,
        include_audio: bool,
    },
}

#[cfg(feature = "execute")]
pub(super) fn embedding_media_kind(
    path: &ObjectPath,
    max_frames: usize,
    include_audio: bool,
) -> flow_like_types::Result<EmbeddingMediaKind> {
    let extension = path
        .extension()
        .filter(|extension| !extension.is_empty())
        .ok_or_else(|| flow_like_types::anyhow!("Embedding files need a supported file extension"))?
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" | "jpe" | "png" | "gif" | "webp" | "tif" | "tiff" | "bmp" | "ico"
        | "avif" | "pnm" | "pbm" | "pgm" | "ppm" | "hdr" | "exr" | "qoi" => {
            Ok(EmbeddingMediaKind::Image)
        }
        "txt" | "md" | "markdown" | "rst" | "csv" | "tsv" | "json" | "jsonl" | "yaml" | "yml"
        | "toml" | "log" | "html" | "htm" | "xml" => Ok(EmbeddingMediaKind::Text),
        "wav" | "wave" | "mp3" | "mp2" | "mp1" | "flac" | "ogg" | "oga" | "opus" | "aac"
        | "m4a" | "aif" | "aiff" => Ok(EmbeddingMediaKind::Audio),
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "ts" | "m2ts" | "mts" => {
            if !(1..=1024).contains(&max_frames) {
                return Err(flow_like_types::anyhow!(
                    "Max Frames must be between 1 and 1024"
                ));
            }
            Ok(EmbeddingMediaKind::Video {
                max_frames,
                include_audio,
            })
        }
        "avi" | "mpg" | "mpeg" => Err(flow_like_types::anyhow!(
            "Video container .{extension} is not supported by the file decoder"
        )),
        _ => Err(flow_like_types::anyhow!(
            "Unsupported embedding file extension .{extension}; use a supported text, image, audio, or video file"
        )),
    }
}

#[cfg(feature = "execute")]
#[derive(Debug)]
pub(super) enum DecodedEmbeddingMedia {
    Text(String),
    Image(flow_like_types::image::DynamicImage),
    Audio(EmbeddingAudio),
    Video {
        frames: Vec<(u64, flow_like_types::image::DynamicImage)>,
        duration_ms: u64,
        audio: Option<EmbeddingAudio>,
    },
}

#[cfg(feature = "execute")]
impl DecodedEmbeddingMedia {
    pub(super) fn into_input(self) -> flow_like_types::Result<EmbeddingInput> {
        let part = match self {
            Self::Text(text) => EmbeddingPart::Text(text),
            Self::Image(image) => EmbeddingPart::Image(Arc::new(image)),
            Self::Audio(audio) => EmbeddingPart::Audio(audio.into_audio()?),
            Self::Video {
                frames,
                duration_ms,
                audio,
            } => EmbeddingPart::Video(VideoInput {
                frames: frames
                    .into_iter()
                    .map(|(timestamp_ms, image)| VideoFrame {
                        timestamp_ms,
                        image: Arc::new(image),
                    })
                    .collect(),
                duration_ms,
                audio: audio.map(EmbeddingAudio::into_audio).transpose()?,
            }),
        };
        Ok(EmbeddingInput {
            parts: vec![part],
            title: None,
        })
    }

    pub(super) async fn into_content(
        self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<EmbeddingContent> {
        let part = match self {
            Self::Text(text) => EmbeddingContentPart::Text { text },
            Self::Image(image) => EmbeddingContentPart::Image {
                image: NodeImage::new(context, image).await,
            },
            Self::Audio(audio) => EmbeddingContentPart::Audio(audio),
            Self::Video {
                frames,
                duration_ms,
                audio,
            } => {
                let mut cached_frames = Vec::with_capacity(frames.len());
                for (timestamp_ms, image) in frames {
                    cached_frames.push(EmbeddingVideoFrame {
                        timestamp_ms,
                        image: NodeImage::new(context, image).await,
                    });
                }
                EmbeddingContentPart::Video(EmbeddingVideo {
                    frames: cached_frames,
                    duration_ms,
                    audio,
                })
            }
        };
        Ok(EmbeddingContent {
            parts: vec![part],
            title: None,
        })
    }
}

#[cfg(feature = "execute")]
pub(super) async fn decode_embedding_media(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    kind: EmbeddingMediaKind,
) -> flow_like_types::Result<DecodedEmbeddingMedia> {
    match kind {
        EmbeddingMediaKind::Text => {
            let bytes = video_utils_rs::read_object_bytes(store, path).await?;
            decode_off_thread(move || {
                let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
                let text = std::str::from_utf8(bytes).map_err(|error| {
                    flow_like_types::anyhow!(
                        "Text embedding files must contain valid UTF-8: {error}"
                    )
                })?;
                Ok(DecodedEmbeddingMedia::Text(text.to_owned()))
            })
            .await
        }
        EmbeddingMediaKind::Image => {
            let bytes = video_utils_rs::read_object_bytes(store, path).await?;
            decode_off_thread(move || {
                let image = flow_like_types::image::load_from_memory(&bytes).map_err(|error| {
                    flow_like_types::anyhow!("Could not decode embedding image: {error}")
                })?;
                Ok(DecodedEmbeddingMedia::Image(image))
            })
            .await
        }
        EmbeddingMediaKind::Audio => {
            let bytes = video_utils_rs::read_object_bytes(store, path).await?;
            let path = path.clone();
            decode_off_thread(move || {
                let frames = decode_embedding_audio_bytes(&bytes, &path)?;
                Ok(DecodedEmbeddingMedia::Audio(audio_content(
                    frames, None, None,
                )?))
            })
            .await
        }
        EmbeddingMediaKind::Video {
            max_frames,
            include_audio,
        } => {
            if !(1..=1024).contains(&max_frames) {
                return Err(flow_like_types::anyhow!(
                    "Max Frames must be between 1 and 1024"
                ));
            }
            let bytes = video_utils_rs::read_object_bytes(store, path).await?;
            let path = path.clone();
            decode_off_thread(move || {
                let demuxed = demux_embedding_video_bytes(&bytes, &path)?;
                if let Some(movie) = mp4_timeline(&bytes, &path)? {
                    validate_mp4_video_edits(&movie, &demuxed)?;
                }
                let DecodedVideo {
                    frames,
                    origin_seconds,
                    duration_ms,
                } = decode_video_content(&demuxed, max_frames)?;
                let audio = if include_audio {
                    Some(audio_content(
                        decode_embedding_audio_bytes(&bytes, &path)?,
                        Some(origin_seconds),
                        Some(duration_ms),
                    )?)
                } else {
                    None
                };
                Ok(DecodedEmbeddingMedia::Video {
                    frames,
                    duration_ms,
                    audio,
                })
            })
            .await
        }
    }
}

#[cfg(feature = "execute")]
fn demux_embedding_video_bytes(
    bytes: &Bytes,
    path: &ObjectPath,
) -> flow_like_types::Result<video_utils_rs::DemuxedMedia> {
    use video_utils_rs::ContainerFormat;

    let format = ContainerFormat::from_path(path)
        .or_else(|| ContainerFormat::from_magic(bytes))
        .ok_or_else(|| flow_like_types::anyhow!("Could not identify the video container"))?;
    let demuxed = match format {
        ContainerFormat::Mp4 | ContainerFormat::QuickTime => {
            if bytes.windows(4).any(|window| window == b"moof") {
                video_utils_rs::demux_fragmented_mp4_bytes(format, bytes)
            } else {
                video_utils_rs::demux_iso_bmff_bytes(format, bytes)
            }
        }
        ContainerFormat::Matroska | ContainerFormat::WebM => {
            video_utils_rs::demux_matroska_bytes(format, bytes)
        }
        ContainerFormat::MpegTs => video_utils_rs::demux_mpeg_ts_bytes(bytes),
        ContainerFormat::Flv => video_utils_rs::demux_flv_bytes(bytes),
        ContainerFormat::Ogg => video_utils_rs::demux_ogg_bytes(bytes),
        ContainerFormat::Wav => video_utils_rs::demux_wav_bytes(bytes),
        ContainerFormat::Aiff => video_utils_rs::demux_aiff_bytes(bytes),
        ContainerFormat::RawElementary => {
            video_utils_rs::demux_elementary_bytes_from_path(path, bytes)
        }
        ContainerFormat::Avi | ContainerFormat::MpegPs => {
            return Err(flow_like_types::anyhow!(
                "Video container {} is not supported by the file decoder",
                format.as_str()
            ));
        }
    };
    Ok(demuxed?)
}

#[cfg(feature = "execute")]
async fn decode_off_thread<T, F>(decode: F) -> flow_like_types::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> flow_like_types::Result<T> + Send + 'static,
{
    #[cfg(not(target_family = "wasm"))]
    {
        tokio::task::spawn_blocking(decode).await?
    }
    #[cfg(target_family = "wasm")]
    {
        decode()
    }
}

#[cfg(feature = "execute")]
struct DecodedVideo {
    frames: Vec<(u64, flow_like_types::image::DynamicImage)>,
    origin_seconds: f64,
    duration_ms: u64,
}

#[cfg(feature = "execute")]
fn decode_video_content(
    demuxed: &video_utils_rs::DemuxedMedia,
    max_frames: usize,
) -> flow_like_types::Result<DecodedVideo> {
    let stream = selected_video_stream(&demuxed.media, None)?;
    let packets = demuxed
        .packets
        .iter()
        .filter(|packet| packet.track_id == stream.track_id)
        .map(|packet| {
            let mut packet = packet.clone();
            if stream.codec == video_utils_rs::CodecId::H264
                && matches!(
                    demuxed.format,
                    video_utils_rs::ContainerFormat::Mp4
                        | video_utils_rs::ContainerFormat::QuickTime
                        | video_utils_rs::ContainerFormat::Matroska
                )
                && let Some(config) = &stream.codec_config
            {
                packet.data = length_prefixed_h264_to_annex_b(&packet.data, config)?;
            }
            Ok(packet)
        })
        .collect::<flow_like_types::Result<Vec<_>>>()?;
    let packets = packets.iter().collect::<Vec<_>>();
    let first = packets
        .first()
        .ok_or_else(|| flow_like_types::anyhow!("Video has no packets"))?;
    let origin_seconds = first.pts_seconds();
    let mut end_seconds = origin_seconds;
    let mut previous = origin_seconds;
    for packet in &packets {
        let timestamp = packet.pts_seconds();
        if !timestamp.is_finite() || timestamp < previous || packet.duration < 0 {
            return Err(flow_like_types::anyhow!(
                "This video needs a decoder that retains reordered presentation timestamps. Supply timestamped frames to Embed Content."
            ));
        }
        previous = timestamp;
        end_seconds = end_seconds.max(timestamp + packet.duration_seconds());
    }
    if !origin_seconds.is_finite() || end_seconds <= origin_seconds {
        return Err(flow_like_types::anyhow!(
            "Video needs a finite, positive duration"
        ));
    }
    let duration_ms = ((end_seconds - origin_seconds) * 1000.0).ceil() as u64;
    let indices = sample_indices(packets.len(), max_frames);
    let sampled = (|| {
        let mut decoder = platform_video_decoder(stream)?;
        let mut sampled = Vec::with_capacity(indices.len());
        let mut next = 0;
        for (index, packet) in packets.iter().enumerate() {
            let mut frames = decoder.decode_packet(packet)?;
            if frames.len() != 1 {
                return Err(flow_like_types::anyhow!(
                    "This decoder does not retain a one-to-one packet/frame timeline. Supply timestamped frames to Embed Content."
                ));
            }
            if indices.get(next) == Some(&index) {
                let frame = frames.pop().expect("one decoded frame");
                let timestamp_ms =
                    ((packet.pts_seconds() - origin_seconds) * 1000.0).round() as u64;
                sampled.push((timestamp_ms, frame_image(frame)?));
                next += 1;
            }
        }
        if !decoder.flush()?.is_empty() {
            return Err(flow_like_types::anyhow!(
                "This decoder emits delayed frames without timestamps. Supply timestamped frames to Embed Content."
            ));
        }
        Ok(sampled)
    })();
    let sampled = match sampled {
        Ok(sampled) => sampled,
        Err(platform_error) if stream.codec == video_utils_rs::CodecId::H264 => {
            decode_h264_timed_frames(stream, &packets, &indices, origin_seconds).map_err(
                |error| {
                    flow_like_types::anyhow!(
                        "Video decoding failed: {platform_error}; H.264 fallback: {error}"
                    )
                },
            )?
        }
        Err(error) => return Err(error),
    };
    Ok(DecodedVideo {
        frames: sampled,
        origin_seconds,
        duration_ms,
    })
}

#[cfg(feature = "execute")]
fn length_prefixed_h264_to_annex_b(data: &[u8], config: &[u8]) -> flow_like_types::Result<Bytes> {
    let length_size = video_utils_rs::bitstream::h264::avcc_length_size(config)?;
    let mut output = Vec::with_capacity(data.len());
    let mut cursor = 0;
    while cursor < data.len() {
        let prefix = data
            .get(cursor..cursor + length_size)
            .ok_or_else(|| flow_like_types::anyhow!("H.264 packet has an incomplete NAL length"))?;
        let length = prefix
            .iter()
            .fold(0usize, |length, byte| (length << 8) | usize::from(*byte));
        cursor += length_size;
        let end = cursor.checked_add(length).ok_or_else(|| {
            flow_like_types::anyhow!("H.264 NAL length overflows the packet offset")
        })?;
        let nal = data
            .get(cursor..end)
            .filter(|nal| !nal.is_empty())
            .ok_or_else(|| flow_like_types::anyhow!("H.264 NAL length is outside the packet"))?;
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(nal);
        cursor = end;
    }
    Ok(Bytes::from(output))
}

#[cfg(feature = "execute")]
fn decode_h264_timed_frames(
    stream: &video_utils_rs::StreamInfo,
    packets: &[&video_utils_rs::EncodedPacket],
    indices: &[usize],
    origin_seconds: f64,
) -> flow_like_types::Result<Vec<(u64, flow_like_types::image::DynamicImage)>> {
    for (packet_index, packet) in packets.iter().enumerate() {
        let annex_b = video_utils_rs::bitstream::h264::h264_packet_to_annex_b(
            packet,
            stream.codec_config.as_ref(),
        )?;
        // The decoder emits pictures in display order, possibly on flush. One
        // picture per packet plus ordered PTS gives each picture an exact timestamp.
        let pictures = annex_b
            .windows(3)
            .enumerate()
            .filter(|(offset, prefix)| {
                *prefix == [0, 0, 1]
                    && annex_b
                        .get(offset + 3)
                        .is_some_and(|header| matches!(header & 0x1f, 1 | 5))
                    && annex_b.get(offset + 4).is_some_and(|byte| byte & 0x80 != 0)
            })
            .count();
        if pictures != 1 {
            return Err(flow_like_types::anyhow!(
                "H.264 timestamp mapping requires exactly one picture per packet (packet {packet_index}, found {pictures})"
            ));
        }
    }
    let mut decoder = match stream.codec_config.as_ref() {
        Some(config) => video_utils_rs::RustH264Decoder::with_avcc_config(config.clone())?,
        None => video_utils_rs::RustH264Decoder::new_annex_b(),
    };
    let mut sampled = Vec::with_capacity(indices.len());
    let mut decoded_count = 0;
    let mut next = 0;
    let mut take_frames = |frames: Vec<video_utils_rs::RgbaFrame>| -> flow_like_types::Result<()> {
        for frame in frames {
            let packet = packets.get(decoded_count).ok_or_else(|| {
                flow_like_types::anyhow!("H.264 decoder emitted more pictures than timed packets")
            })?;
            if indices.get(next) == Some(&decoded_count) {
                let timestamp_ms =
                    ((packet.pts_seconds() - origin_seconds) * 1000.0).round() as u64;
                sampled.push((timestamp_ms, frame_image(frame)?));
                next += 1;
            }
            decoded_count += 1;
        }
        Ok(())
    };
    for packet in packets {
        take_frames(decoder.decode_packet(packet)?)?;
    }
    take_frames(decoder.flush()?)?;
    if decoded_count != packets.len() {
        return Err(flow_like_types::anyhow!(
            "H.264 decoder emitted fewer pictures than timed packets"
        ));
    }
    Ok(sampled)
}

#[cfg(all(feature = "execute", test))]
async fn decode_embedding_audio(
    store: &dyn ObjectStore,
    path: &ObjectPath,
) -> flow_like_types::Result<Vec<video_utils_rs::AudioFrame>> {
    let bytes = video_utils_rs::read_object_bytes(store, path).await?;
    decode_embedding_audio_bytes(&bytes, path)
}

#[cfg(feature = "execute")]
fn decode_embedding_audio_bytes(
    bytes: &Bytes,
    path: &ObjectPath,
) -> flow_like_types::Result<Vec<video_utils_rs::AudioFrame>> {
    use symphonia::core::{
        audio::SampleBuffer, codecs::DecoderOptions, errors::Error as AudioError,
        formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
        units::TimeBase,
    };

    let movie = mp4_timeline(bytes, path)?;
    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.clone())),
        Default::default(),
    );
    let mut hint = Hint::new();
    if let Some(extension) = path.extension() {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?
        .format;
    let track = format
        .default_track()
        .filter(|track| track.codec_params.sample_rate.is_some())
        .or_else(|| {
            format
                .tracks()
                .iter()
                .find(|track| track.codec_params.sample_rate.is_some())
        })
        .ok_or_else(|| flow_like_types::anyhow!("Media has no audio track"))?;
    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut frames = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(AudioError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        let time_base = time_base.unwrap_or_else(|| TimeBase::new(1, spec.rate));
        let scaled = u128::from(packet.ts()) * u128::from(time_base.numer) * u128::from(spec.rate);
        let pts = i64::try_from(
            (scaled + u128::from(time_base.denom / 2)) / u128::from(time_base.denom),
        )?;
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        if !samples.is_empty() {
            frames.push(video_utils_rs::AudioFrame::new(
                spec.rate,
                u16::try_from(spec.channels.count())?,
                pts,
                samples.samples().to_vec(),
            )?);
        }
    }
    if let Some(movie) = movie {
        apply_mp4_audio_edits(frames, &movie, track_id as usize)
    } else {
        Ok(frames)
    }
}

#[cfg(feature = "execute")]
fn mp4_timeline(bytes: &Bytes, path: &ObjectPath) -> flow_like_types::Result<Option<re_mp4::Mp4>> {
    let is_mp4 = bytes.get(4..8) == Some(b"ftyp")
        || path.extension().is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp4" | "m4a" | "m4b" | "m4v" | "mov"
            )
        });
    if is_mp4 {
        Ok(Some(re_mp4::Mp4::read_bytes(bytes)?))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "execute")]
fn validate_mp4_video_edits(
    movie: &re_mp4::Mp4,
    demuxed: &video_utils_rs::DemuxedMedia,
) -> flow_like_types::Result<()> {
    let stream = selected_video_stream(&demuxed.media, None)?;
    let track = movie
        .tracks()
        .get(&stream.track_id)
        .ok_or_else(|| flow_like_types::anyhow!("MP4 video track has no timing metadata"))?;
    let track = track.trak(movie);
    if let Some(edits) = track.edts.as_ref().and_then(|edits| edits.elst.as_ref())
        && !edits.entries.is_empty()
    {
        let entry = &edits.entries[0];
        let edit_duration =
            u128::from(entry.segment_duration) * u128::from(track.mdia.mdhd.timescale);
        let media_duration =
            u128::from(track.mdia.mdhd.duration) * u128::from(movie.moov.mvhd.timescale);
        // Movie durations round to movie ticks, which can be coarser than the video track.
        let full_duration =
            edit_duration.abs_diff(media_duration) <= u128::from(track.mdia.mdhd.timescale);
        if edits.entries.len() != 1
            || entry.media_time != 0
            || entry.media_rate != 1
            || entry.media_rate_fraction != 0
            || !full_duration
        {
            return Err(flow_like_types::anyhow!(
                "This MP4 shifts or trims its video track through an edit list. Supply timestamped frames to Embed Content."
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn apply_mp4_audio_edits(
    frames: Vec<video_utils_rs::AudioFrame>,
    movie: &re_mp4::Mp4,
    track_index: usize,
) -> flow_like_types::Result<Vec<video_utils_rs::AudioFrame>> {
    let track = movie
        .moov
        .traks
        .get(track_index)
        .ok_or_else(|| flow_like_types::anyhow!("MP4 audio track has no timing metadata"))?;
    let Some(edits) = track.edts.as_ref().and_then(|edits| edits.elst.as_ref()) else {
        return Ok(frames);
    };
    if edits.entries.is_empty() || frames.is_empty() {
        return Ok(frames);
    }
    let sample_rate = frames[0].sample_rate;
    let scale = |ticks: u64, denominator: u32| -> flow_like_types::Result<i64> {
        if denominator == 0 {
            return Err(flow_like_types::anyhow!("MP4 timescale must be positive"));
        }
        Ok(i64::try_from(
            (u128::from(ticks) * u128::from(sample_rate) + u128::from(denominator / 2))
                / u128::from(denominator),
        )?)
    };
    let mut output = Vec::new();
    let mut output_start = 0i64;
    for entry in &edits.entries {
        if entry.media_rate != 1 || entry.media_rate_fraction != 0 {
            return Err(flow_like_types::anyhow!(
                "MP4 audio edit lists must use normal playback speed"
            ));
        }
        let duration = scale(entry.segment_duration, movie.moov.mvhd.timescale)?;
        let output_end = output_start
            .checked_add(duration)
            .ok_or_else(|| flow_like_types::anyhow!("MP4 audio edit timeline is too large"))?;
        let empty = entry.media_time == u64::MAX
            || (edits.version == 0 && entry.media_time == u64::from(u32::MAX));
        if !empty {
            let input_start = scale(entry.media_time, track.mdia.mdhd.timescale)?;
            let input_end = input_start
                .checked_add(duration)
                .ok_or_else(|| flow_like_types::anyhow!("MP4 audio edit duration is too large"))?;
            for frame in &frames {
                if frame.sample_rate != sample_rate {
                    return Err(flow_like_types::anyhow!(
                        "Audio sample rate changes within a track"
                    ));
                }
                let start = frame.pts.max(input_start);
                let end = frame.end_pts().min(input_end);
                if start >= end {
                    continue;
                }
                let channels = usize::from(frame.channels);
                let skip = usize::try_from(start - frame.pts)? * channels;
                let length = usize::try_from(end - start)? * channels;
                output.push(video_utils_rs::AudioFrame::new(
                    sample_rate,
                    frame.channels,
                    output_start + (start - input_start),
                    frame.samples_f32_interleaved[skip..skip + length].to_vec(),
                )?);
            }
        }
        output_start = output_end;
    }
    Ok(output)
}

#[cfg(any(feature = "execute", test))]
fn sample_indices(total: usize, maximum: usize) -> Vec<usize> {
    let count = total.min(maximum);
    match count {
        0 => Vec::new(),
        1 => vec![0],
        _ => (0..count)
            .map(|index| index * (total - 1) / (count - 1))
            .collect(),
    }
}

#[cfg(feature = "execute")]
fn frame_image(
    frame: video_utils_rs::RgbaFrame,
) -> flow_like_types::Result<flow_like_types::image::DynamicImage> {
    let row_bytes = frame.width as usize * 4;
    let mut packed = Vec::with_capacity(row_bytes * frame.height as usize);
    for row in 0..frame.height as usize {
        packed.extend_from_slice(&frame.data[row * frame.stride..row * frame.stride + row_bytes]);
    }
    let image = flow_like_types::image::RgbaImage::from_raw(frame.width, frame.height, packed)
        .ok_or_else(|| flow_like_types::anyhow!("Decoded frame has invalid dimensions"))?;
    Ok(flow_like_types::image::DynamicImage::ImageRgba8(image))
}

#[cfg(feature = "execute")]
fn audio_content(
    frames: Vec<video_utils_rs::AudioFrame>,
    origin_seconds: Option<f64>,
    duration_ms: Option<u64>,
) -> flow_like_types::Result<EmbeddingAudio> {
    let first = frames
        .first()
        .ok_or_else(|| flow_like_types::anyhow!("Media has no decoded audio samples"))?;
    let sample_rate = first.sample_rate;
    let channels = first.channels;
    let origin = origin_seconds
        .map(|seconds| (seconds * f64::from(sample_rate)).round() as i64)
        .unwrap_or(first.pts);
    let limit = duration_ms
        .map(|ms| (u128::from(ms) * u128::from(sample_rate) / 1000).min(i64::MAX as u128) as i64);
    let mut samples = Vec::new();
    let mut previous_end = i64::MIN;
    for frame in frames {
        if frame.sample_rate != sample_rate
            || frame.channels != channels
            || frame.pts < previous_end
        {
            return Err(flow_like_types::anyhow!(
                "Audio must have one sample rate and channel layout with non-overlapping frames"
            ));
        }
        previous_end = frame.end_pts();
        let start = frame.pts.saturating_sub(origin);
        let end = previous_end.saturating_sub(origin);
        let clipped_start = start.max(0);
        let clipped_end = limit.map_or(end, |limit| end.min(limit));
        if clipped_start >= clipped_end {
            continue;
        }
        let channels = usize::from(channels);
        let dest_start = usize::try_from(clipped_start)?
            .checked_mul(channels)
            .ok_or_else(|| flow_like_types::anyhow!("Audio is too long"))?;
        let skip = usize::try_from(clipped_start - start)?
            .checked_mul(channels)
            .ok_or_else(|| flow_like_types::anyhow!("Audio is too long"))?;
        let length = usize::try_from(clipped_end - clipped_start)?
            .checked_mul(channels)
            .ok_or_else(|| flow_like_types::anyhow!("Audio is too long"))?;
        if samples.len() < dest_start {
            samples.resize(dest_start, 0.0);
        }
        samples.extend_from_slice(&frame.samples_f32_interleaved[skip..skip + length]);
    }
    let audio = EmbeddingAudio {
        samples,
        sample_rate,
        channels,
    };
    audio.validate()?;
    Ok(audio)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "execute")]
    use flow_like_storage::object_store::ObjectStoreExt;

    #[test]
    fn frame_sampling_keeps_endpoints_without_duplicates() {
        assert_eq!(sample_indices(10, 4), [0, 3, 6, 9]);
        assert_eq!(sample_indices(2, 16), [0, 1]);
        assert_eq!(sample_indices(10, 1), [0]);
        assert!(sample_indices(0, 16).is_empty());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn classifies_extensions_without_guessing_unknown_file_types() {
        assert_eq!(
            embedding_media_kind(&ObjectPath::from("media/PHOTO.JpEg"), 16, false).unwrap(),
            EmbeddingMediaKind::Image
        );
        assert_eq!(
            embedding_media_kind(&ObjectPath::from("notes.MD"), 16, false).unwrap(),
            EmbeddingMediaKind::Text
        );
        assert_eq!(
            embedding_media_kind(&ObjectPath::from("voice.WaV"), 16, false).unwrap(),
            EmbeddingMediaKind::Audio
        );
        assert_eq!(
            embedding_media_kind(&ObjectPath::from("clip.MP4"), 3, true).unwrap(),
            EmbeddingMediaKind::Video {
                max_frames: 3,
                include_audio: true,
            }
        );
        for path in ["binary.bin", "unknown", "clip.avi", "clip.mpg", "clip.mpeg"] {
            assert!(embedding_media_kind(&ObjectPath::from(path), 16, false).is_err());
        }
        for max_frames in [0, 1025] {
            assert!(
                embedding_media_kind(&ObjectPath::from("clip.mp4"), max_frames, false).is_err()
            );
        }
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn decodes_mixed_text_and_image_files_into_their_content_parts() {
        let store = flow_like_storage::object_store::memory::InMemory::new();
        let text_path = ObjectPath::from("notes.MD");
        let image_path = ObjectPath::from("photo.PNG");
        store
            .put(
                &text_path,
                Bytes::from_static("\u{feff}# Café\n  keep whitespace\n".as_bytes()).into(),
            )
            .await
            .unwrap();
        let image = flow_like_types::image::DynamicImage::ImageRgb8(
            flow_like_types::image::RgbImage::from_pixel(
                2,
                3,
                flow_like_types::image::Rgb([3, 31, 127]),
            ),
        );
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, flow_like_types::image::ImageFormat::Png)
            .unwrap();
        store
            .put(&image_path, Bytes::from(encoded.into_inner()).into())
            .await
            .unwrap();
        let mut inputs = Vec::new();
        for path in [&text_path, &image_path] {
            let kind = embedding_media_kind(path, 16, false).unwrap();
            inputs.push(
                decode_embedding_media(&store, path, kind)
                    .await
                    .unwrap()
                    .into_input()
                    .unwrap(),
            );
        }
        assert!(inputs.iter().all(|input| input.title.is_none()));
        let EmbeddingPart::Text(text) = &inputs[0].parts[0] else {
            panic!("Markdown must become a text part");
        };
        assert_eq!(text, "# Café\n  keep whitespace\n");
        let EmbeddingPart::Image(image) = &inputs[1].parts[0] else {
            panic!("PNG must become an image part");
        };
        assert_eq!((image.width(), image.height()), (2, 3));
        assert_eq!(image.to_rgb8().get_pixel(1, 2).0, [3, 31, 127]);
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn rejects_invalid_utf8_and_invalid_image_bytes() {
        let store = flow_like_storage::object_store::memory::InMemory::new();
        for (name, bytes, error_fragment) in [
            ("invalid.md", &[0xef, 0xbb, 0xbf, 0xff][..], "valid UTF-8"),
            (
                "invalid.png",
                &b"not an image"[..],
                "decode embedding image",
            ),
        ] {
            let path = ObjectPath::from(name);
            store
                .put(&path, Bytes::copy_from_slice(bytes).into())
                .await
                .unwrap();
            let kind = embedding_media_kind(&path, 16, false).unwrap();
            let error = decode_embedding_media(&store, &path, kind)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(error_fragment), "{error}");
        }
    }

    #[cfg(feature = "execute")]
    #[test]
    fn decoded_audio_retains_gaps_and_clips_to_video_timeline() {
        let frames = vec![
            video_utils_rs::AudioFrame::new(1000, 1, 0, vec![0.1, 0.2]).unwrap(),
            video_utils_rs::AudioFrame::new(1000, 1, 4, vec![0.3, 0.4]).unwrap(),
        ];
        assert_eq!(
            audio_content(frames.clone(), None, None).unwrap().samples,
            [0.1, 0.2, 0.0, 0.0, 0.3, 0.4]
        );
        assert_eq!(
            audio_content(frames, Some(0.001), Some(4)).unwrap().samples,
            [0.2, 0.0, 0.0, 0.3]
        );
    }

    #[cfg(feature = "execute")]
    #[test]
    fn decoded_frames_respect_padded_row_stride() {
        let frame = video_utils_rs::RgbaFrame::new(
            1,
            2,
            8,
            vec![1, 2, 3, 255, 0, 0, 0, 0, 4, 5, 6, 255, 0, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(
            frame_image(frame).unwrap().to_rgba8().as_raw(),
            &[1, 2, 3, 255, 4, 5, 6, 255]
        );
    }

    #[cfg(feature = "execute")]
    #[test]
    fn h264_nal_lengths_cannot_be_mistaken_for_start_codes() {
        let mut packet = vec![0, 0, 1, 0];
        packet.extend_from_slice(&[0x41, 0x80]);
        packet.resize(260, 0);
        let normalized = length_prefixed_h264_to_annex_b(&packet, &[1, 0, 0, 0, 0xff]).unwrap();
        assert_eq!(&normalized[..6], &[0, 0, 0, 1, 0x41, 0x80]);
        assert_eq!(normalized.len(), packet.len());
        assert!(length_prefixed_h264_to_annex_b(&packet[..259], &[1, 0, 0, 0, 0xff]).is_err());
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn decodes_a_real_wav_file_into_embedding_samples() {
        let store = flow_like_storage::object_store::memory::InMemory::new();
        let path = ObjectPath::from("embedding-tone.wav");
        let bytes =
            Bytes::from_static(include_bytes!("../../../tests/fixtures/embedding-tone.wav"));
        store.put(&path, bytes.into()).await.unwrap();
        let frames = decode_embedding_audio(&store, &path).await.unwrap();
        let audio = audio_content(frames, None, None).unwrap();
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.samples.len(), 16_000);
        let rms = (audio.samples.iter().map(|value| value * value).sum::<f32>() / 16_000.0).sqrt();
        assert!((0.08..0.10).contains(&rms), "Decoded tone RMS: {rms}");
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn decodes_a_real_low_delay_mp4_with_exact_sample_timestamps() {
        let store = flow_like_storage::object_store::memory::InMemory::new();
        let path = ObjectPath::from("embedding-lowdelay.mp4");
        let bytes = Bytes::from_static(include_bytes!(
            "../../../tests/fixtures/embedding-lowdelay.mp4"
        ));
        store.put(&path, bytes.clone().into()).await.unwrap();
        let demuxed = video_utils_rs::demux_object(&store, &path).await.unwrap();
        let mut movie = mp4_timeline(&bytes, &path).unwrap().unwrap();
        validate_mp4_video_edits(&movie, &demuxed).unwrap();
        let video = decode_video_content(&demuxed, 3).unwrap();
        assert_eq!(video.duration_ms, 1000);
        assert_eq!(
            video.frames.iter().map(|frame| frame.0).collect::<Vec<_>>(),
            [0, 400, 800]
        );
        assert!(
            video
                .frames
                .iter()
                .all(|(_, image)| image.width() == 64 && image.height() == 48)
        );
        assert_ne!(video.frames[0].1.as_bytes(), video.frames[2].1.as_bytes());
        let original_duration = movie.moov.traks[0].mdia.mdhd.duration;
        let one_movie_tick = movie.moov.traks[0].mdia.mdhd.timescale / movie.moov.mvhd.timescale;
        movie.moov.traks[0].mdia.mdhd.duration += u64::from(one_movie_tick);
        validate_mp4_video_edits(&movie, &demuxed).unwrap();
        movie.moov.traks[0].mdia.mdhd.duration += 1;
        assert!(validate_mp4_video_edits(&movie, &demuxed).is_err());
        movie.moov.traks[0].mdia.mdhd.duration = original_duration;
        movie.moov.traks[0]
            .edts
            .as_mut()
            .unwrap()
            .elst
            .as_mut()
            .unwrap()
            .entries[0]
            .media_time = 1;
        assert!(validate_mp4_video_edits(&movie, &demuxed).is_err());
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn video_audio_keeps_its_offset_on_the_video_timeline() {
        let store = flow_like_storage::object_store::memory::InMemory::new();
        let path = ObjectPath::from("embedding-offset-audio.mp4");
        let bytes = Bytes::from_static(include_bytes!(
            "../../../tests/fixtures/embedding-offset-audio.mp4"
        ));
        store.put(&path, bytes.clone().into()).await.unwrap();
        let demuxed = video_utils_rs::demux_object(&store, &path).await.unwrap();
        let movie = mp4_timeline(&bytes, &path).unwrap().unwrap();
        validate_mp4_video_edits(&movie, &demuxed).unwrap();
        let video = decode_video_content(&demuxed, 3).unwrap();
        let frames = decode_embedding_audio(&store, &path).await.unwrap();
        let first_pts = frames[0].pts;
        assert_eq!(first_pts, 2176);
        let audio =
            audio_content(frames, Some(video.origin_seconds), Some(video.duration_ms)).unwrap();
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.samples.len(), 14_400);
        assert!(
            audio.samples[..first_pts as usize]
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(audio.samples.iter().any(|sample| sample.abs() > 0.05));
    }
}
