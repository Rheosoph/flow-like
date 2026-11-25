use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::{
    AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedReader,
    SharedSource, VideoStreamInfo,
};
use filmcraft_time::{FrameRate, Tick};
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/browser-codecs.js")]
extern "C" {
    #[wasm_bindgen(js_name = onSourceReady)]
    fn on_source_ready(callback: &js_sys::Function);
    #[wasm_bindgen(catch, js_name = openSource)]
    fn open_source(bytes: &[u8], name: &str, metadata: &str) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_name = frame)]
    fn read_frame(id: u32, seconds: f64) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = audio)]
    fn read_audio(id: u32, start: f64, frames: u32, rate: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = closeSource)]
    fn close_source(id: u32);
}

thread_local! {
    static SOURCES: RefCell<HashMap<String, SharedSource>> = RefCell::new(HashMap::new());
    static DECODE_FAILURE: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn take_failure() -> Option<String> {
    DECODE_FAILURE.with(|failure| failure.borrow_mut().take())
}

fn decode_failure(message: String) -> MediaError {
    DECODE_FAILURE.with(|failure| *failure.borrow_mut() = Some(message.clone()));
    MediaError::Unsupported(message)
}

pub(crate) fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}
pub(crate) fn error(error: JsValue) -> String {
    field(&error, "message")
        .as_string()
        .or_else(|| error.as_string())
        .unwrap_or_else(|| format!("Browser media error: {error:?}"))
}

pub async fn probe() {
    crate::set_info(
        "webcodecs",
        json!({"backend": "browser", "softwareFallback": false}),
    );
}
pub fn install() {
    filmcraft_codecs::register_reader_opener(reader_opener);
    let callback = Closure::wrap(Box::new(crate::repaint) as Box<dyn FnMut()>);
    on_source_ready(callback.as_ref().unchecked_ref());
    callback.forget();
}
pub fn stats() -> serde_json::Value {
    json!({"backend": "browser", "softwareFallback": false})
}

pub async fn prepare(name: &str, bytes: &[u8]) -> Result<(), String> {
    let mut metadata = json!({});
    if filmcraft_codecs::mp4::sniff(bytes) {
        let file = filmcraft_isobmff::open(bytes)
            .map_err(|e| format!("The MP4 container could not be read: {e}"))?;
        metadata["audio"] = json!(
            file.tracks
                .iter()
                .any(|t| t.kind == filmcraft_isobmff::TrackKind::Audio && !t.samples.is_empty())
        );
        if let Some(track) = file
            .tracks
            .iter()
            .find(|t| t.kind == filmcraft_isobmff::TrackKind::Video && !t.samples.is_empty())
        {
            let mut durations: Vec<u32> = track
                .samples
                .iter()
                .take(240)
                .map(|s| s.duration)
                .filter(|d| *d > 0)
                .collect();
            durations.sort_unstable();
            if let Some(duration) = durations.get(durations.len() / 2) {
                metadata["frameRate"] = json!(track.timescale as f64 / *duration as f64);
            }
        }
    } else if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let file = filmcraft_matroska::open(bytes)
            .map_err(|e| format!("The WebM/Matroska container could not be read: {e}"))?;
        metadata["audio"] = json!(
            file.tracks
                .iter()
                .any(|t| t.kind == filmcraft_matroska::TrackKind::Audio && !t.samples.is_empty())
        );
        if let Some(duration) = file
            .tracks
            .iter()
            .find(|t| t.kind == filmcraft_matroska::TrackKind::Video)
            .and_then(|t| t.default_duration_ns)
        {
            metadata["frameRate"] = json!(1_000_000_000.0 / duration as f64);
        }
    }
    let value = JsFuture::from(open_source(bytes, name, &metadata.to_string()).map_err(error)?)
        .await
        .map_err(error)?;
    let number = |key: &str| field(&value, key).as_f64().unwrap_or_default();
    let id = number("id") as u32;
    let channels = number("channels") as u32;
    let source: SharedSource = Arc::new(BrowserSource {
        id,
        info: MediaInfo {
            name: name.into(),
            kind: MediaKind::Movie,
            duration: Tick::from_seconds_f64(number("duration")),
            video: Some(VideoStreamInfo {
                width: number("width") as u32,
                height: number("height") as u32,
                frame_rate: FrameRate::from_f64(number("frameRate")),
                par: (1, 1),
                codec: "Browser decoder".into(),
                pixel_format: "RGBA8".into(),
                color: filmcraft_color::ColorInfo::REC709,
                has_alpha: false,
                bitrate: None,
                hdr: None,
                color_assumed: None,
            }),
            audio_streams: if channels > 0 {
                vec![AudioStreamInfo {
                    sample_rate: number("sampleRate") as u32,
                    channels,
                    codec: "Browser decoder".into(),
                    bits_per_sample: Some(32),
                }]
            } else {
                Vec::new()
            },
            container: "Browser media".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        },
    });
    SOURCES.with(|sources| {
        sources.borrow_mut().insert(name.into(), source);
    });
    Ok(())
}

fn reader_opener(
    name: &str,
    _head: &[u8],
    _reader: &SharedReader,
) -> Option<Result<SharedSource, MediaError>> {
    SOURCES.with(|sources| sources.borrow().get(name).cloned().map(Ok))
}

struct BrowserSource {
    id: u32,
    info: MediaInfo,
}
impl Drop for BrowserSource {
    fn drop(&mut self) {
        close_source(self.id);
    }
}
impl MediaSource for BrowserSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, request: FrameRequest) -> Result<Arc<VideoFrame>, MediaError> {
        let info = self
            .info
            .video
            .as_ref()
            .ok_or(MediaError::NoStream("video"))?;
        let pixels =
            read_frame(self.id, request.time.seconds()).map_err(|e| decode_failure(error(e)))?;
        if pixels.is_null() || pixels.is_undefined() {
            filmcraft_media::pending::mark();
            crate::repaint();
            return Err(MediaError::Decode(
                "The browser is decoding this frame.".into(),
            ));
        }
        let pixels = js_sys::Uint8Array::new(&pixels).to_vec();
        if pixels.len() != info.width as usize * info.height as usize * 4 {
            return Err(decode_failure(
                "The browser returned an incomplete frame.".into(),
            ));
        }
        Ok(Arc::new(
            VideoFrame::rgba8(info.width, info.height, pixels).with_pts(request.time),
        ))
    }
    fn audio(
        &self,
        start: i64,
        frames: usize,
        sample_rate: u32,
    ) -> Result<AudioBuffer, MediaError> {
        if self.info.audio_streams.is_empty() {
            return Err(MediaError::NoStream("audio"));
        }
        let value = read_audio(self.id, start as f64, frames as u32, sample_rate)
            .map_err(|e| decode_failure(error(e)))?;
        let channels: Vec<Vec<f32>> = js_sys::Array::from(&value)
            .iter()
            .map(|v| js_sys::Float32Array::new(&v).to_vec())
            .collect();
        if channels.len() != self.info.audio_streams[0].channels as usize
            || channels.iter().any(|channel| channel.len() != frames)
        {
            return Err(decode_failure(
                "The browser returned incomplete audio.".into(),
            ));
        }
        Ok(AudioBuffer {
            sample_rate,
            channels,
        })
    }
}
