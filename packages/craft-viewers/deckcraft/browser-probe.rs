use crate::{Bytes, Container, MediaError, Result, browser};
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub decodable: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: u64,
    pub decodable: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub container: &'static str,
    pub duration_ms: u64,
    pub audio: Option<AudioInfo>,
    pub video: Option<VideoInfo>,
}
impl MediaInfo {
    pub fn duration(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }
    pub fn playable(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.decodable)
            || self.video.as_ref().is_some_and(|v| v.decodable)
    }
}
pub fn probe(bytes: &Bytes) -> Result<MediaInfo> {
    let value = browser::probe_js(&browser::asset(bytes)?);
    if let Some(error) = browser::field(&value, "error").as_string() {
        return Err(MediaError::Unsupported(error));
    }
    if browser::field(&value, "pending").as_bool() == Some(true) {
        return Err(MediaError::Corrupt(
            "The browser is loading this media.".into(),
        ));
    }
    let width = browser::number(&value, "width") as u32;
    let height = browser::number(&value, "height") as u32;
    Ok(MediaInfo {
        container: Container::sniff(bytes).name(),
        duration_ms: (browser::number(&value, "duration") * 1000.0).round() as u64,
        audio: Some(AudioInfo {
            codec: "Browser audio".into(),
            sample_rate: 0,
            channels: 0,
            decodable: true,
        }),
        video: (width > 0 && height > 0).then(|| VideoInfo {
            codec: "Browser video".into(),
            width,
            height,
            fps: 0.0,
            frames: 0,
            decodable: true,
        }),
    })
}

// Browser probing reads metadata without decoding the full stream.
pub fn probe_headers(bytes: &Bytes) -> Result<MediaInfo> {
    probe(bytes)
}
