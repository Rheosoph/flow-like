use crate::webcodecs::{error, field};
use filmcraft_engine::{MediaPool, Services};
use filmcraft_isobmff::{
    AvcConfig, Brand, Edit, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use filmcraft_project::{ItemId, Project};
use filmcraft_time::Tick;
use std::cell::RefCell;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/browser-codecs.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = addAacRollGroups)]
    fn add_aac_roll_groups(bytes: &[u8]) -> Result<Vec<u8>, JsValue>;
    #[wasm_bindgen(js_name = setCalibrationMuxer)]
    fn set_calibration_muxer(callback: &js_sys::Function);
    #[wasm_bindgen(catch, js_name = createEncoder)]
    fn create_encoder(
        width: u32,
        height: u32,
        rate: f64,
        sample_rate: u32,
        channels: u32,
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_name = encodeFrame)]
    fn encode_frame(
        id: u32,
        pixels: &[u8],
        width: u32,
        height: u32,
        timestamp: f64,
        duration: f64,
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_name = encodeAudio)]
    fn encode_audio(
        id: u32,
        planar: &[f32],
        count: u32,
        timestamp: f64,
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_name = finishEncoder)]
    fn finish_encoder(id: u32) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(js_name = cancelEncoder)]
    fn cancel_encoder(id: u32);
}

pub fn install() {
    let callback = Closure::wrap(Box::new(|probe: JsValue| -> JsValue {
        match mux_audio_probe(probe) {
            Ok(bytes) => js_sys::Uint8Array::from(bytes.as_slice()).into(),
            Err(error) => JsValue::from_str(&error),
        }
    }) as Box<dyn FnMut(JsValue) -> JsValue>);
    set_calibration_muxer(callback.as_ref().unchecked_ref());
    callback.forget();
}

fn mux_audio_probe(probe: JsValue) -> Result<Vec<u8>, String> {
    let sample_rate = field(&probe, "sampleRate").as_f64().unwrap_or(0.0) as u32;
    let channels = field(&probe, "channels").as_f64().unwrap_or(0.0) as u32;
    if sample_rate == 0 || channels == 0 {
        return Err("Invalid audio calibration configuration.".into());
    }
    let description = js_sys::Uint8Array::new(&field(&probe, "description")).to_vec();
    let packets = js_sys::Array::from(&field(&probe, "packets"));
    if packets.length() == 0 || packets.length() > 64 {
        return Err("Invalid audio calibration packet count.".into());
    }
    let frames = |packet: &JsValue| {
        (field(packet, "duration")
            .as_f64()
            .unwrap_or(1024.0 * 1_000_000.0 / sample_rate as f64)
            * sample_rate as f64
            / 1_000_000.0)
            .round() as u32
    };
    let total: u64 = packets.iter().map(|packet| frames(&packet) as u64).sum();
    let mut options = WriterOptions::new(Brand::Mp4);
    options.movie_timescale = sample_rate;
    let io = |error: filmcraft_isobmff::Error| error.to_string();
    let mut writer = Mp4Writer::new(std::io::Cursor::new(Vec::new()), options).map_err(io)?;
    let mut config = TrackConfig::new(
        SampleEntry::aac(description, channels, sample_rate),
        sample_rate,
    );
    config.edits = vec![Edit {
        segment_duration: total,
        media_time: 0,
        media_rate: 0x0001_0000,
    }];
    let track = writer.add_track(config).map_err(io)?;
    for packet in packets.iter() {
        let bytes = js_sys::Uint8Array::new(&field(&packet, "data")).to_vec();
        writer
            .write_sample(
                track,
                WriteSample {
                    data: &bytes,
                    duration: frames(&packet),
                    composition_offset: 0,
                    is_sync: true,
                },
            )
            .map_err(io)?;
    }
    add_aac_roll_groups(&writer.finish_faststart().map_err(io)?.into_inner()).map_err(error)
}

thread_local! { static RESULT: RefCell<Option<Result<Vec<u8>, String>>> = const { RefCell::new(None) }; }
pub fn take_result() -> Option<Result<Vec<u8>, String>> {
    RESULT.with(|result| result.borrow_mut().take())
}

pub fn start(
    project: Arc<Project>,
    sequence: ItemId,
    pool: Arc<MediaPool>,
    services: Arc<dyn Services>,
    progress: impl Fn(Option<f64>, &'static str) + 'static,
) {
    RESULT.with(|result| *result.borrow_mut() = None);
    wasm_bindgen_futures::spawn_local(async move {
        let result = export(project, sequence, pool, services, &progress).await;
        RESULT.with(|slot| *slot.borrow_mut() = Some(result));
        crate::repaint();
    });
}

struct EncoderGuard(u32);
impl Drop for EncoderGuard {
    fn drop(&mut self) {
        cancel_encoder(self.0);
    }
}

async fn export(
    project: Arc<Project>,
    sequence: ItemId,
    pool: Arc<MediaPool>,
    services: Arc<dyn Services>,
    progress: &dyn Fn(Option<f64>, &'static str),
) -> Result<Vec<u8>, String> {
    progress(None, "preparing");
    let seq = project
        .sequence(sequence)
        .ok_or("There is no active video sequence.")?;
    let rate = seq.settings.frame_rate;
    let width = seq.settings.width;
    let height = seq.settings.height;
    let sample_rate = seq.settings.sample_rate;
    let duration = seq.duration();
    if duration.0 <= 0 {
        return Err("The video sequence is empty.".into());
    }
    let total = (rate.frame_at(duration - Tick(1)) + 1).max(1);
    let channels = if seq.audio_tracks.iter().any(|track| !track.items.is_empty()) {
        2
    } else {
        0
    };
    let value = JsFuture::from(
        create_encoder(
            width,
            height,
            rate.num as f64 / rate.den as f64,
            sample_rate,
            channels,
        )
        .map_err(error)?,
    )
    .await
    .map_err(error)?;
    let id = value
        .as_f64()
        .ok_or("The browser did not create a video encoder.")? as u32;
    let _encoder = EncoderGuard(id);
    let provider = pool
        .full_res_provider(project.clone(), services)
        .recording();
    let mut audio_start = 0i64;
    crate::webcodecs::take_failure();
    progress(Some(0.0), "rendering");
    let mut last_progress_at = web_time::Instant::now();
    for index in 0..total {
        let time = rate.tick_of(index);
        let mut ready = None;
        let started = web_time::Instant::now();
        while started.elapsed().as_secs() < 30 {
            filmcraft_media::pending::take();
            let image = filmcraft_render::render_sequence(
                &project,
                sequence,
                time,
                filmcraft_render::RenderOptions {
                    captions: true,
                    ..Default::default()
                },
                &provider,
            );
            if let Some(error) = crate::webcodecs::take_failure() {
                return Err(format!("The video was not saved: {error}"));
            }
            if !filmcraft_media::pending::take() {
                if !provider.offline_used().is_empty() {
                    return Err("Some video frames could not be decoded by the browser. The video was not saved.".into());
                }
                ready = Some(image);
                break;
            }
            crate::fs::sleep(10).await;
        }
        let image = ready
            .ok_or("The browser timed out decoding a video frame. The video was not saved.")?;
        if image.w != width as usize || image.h != height as usize {
            return Err("The rendered video size does not match the sequence.".into());
        }
        let timestamp = (time.seconds() * 1_000_000.0).round();
        let end = rate.tick_of(index + 1).min(duration);
        let length = (end.seconds() * 1_000_000.0).round() - timestamp;
        JsFuture::from(
            encode_frame(
                id,
                &image.to_rgba8(),
                width,
                height,
                timestamp,
                length.max(1.0),
            )
            .map_err(error)?,
        )
        .await
        .map_err(error)?;
        if channels > 0 {
            let mut audio_end = end.to_units_floor(sample_rate as i64);
            // Safari resets AAC packet timestamps at unaligned input boundaries.
            // Keep complete AAC frames together, then send the remaining samples once.
            if index + 1 < total {
                audio_end -= audio_end % 1024;
            }
            while audio_start < audio_end {
                let count = (audio_end - audio_start).min(4096) as usize;
                filmcraft_media::pending::take();
                let buffer = filmcraft_render::audio::mix_sequence(
                    &project,
                    seq,
                    audio_start,
                    count,
                    &provider,
                );
                if let Some(error) = crate::webcodecs::take_failure() {
                    return Err(format!("The video was not saved: {error}"));
                }
                if filmcraft_media::pending::take() || !provider.offline_used().is_empty() {
                    return Err(
                        "Some audio could not be decoded by the browser. The video was not saved."
                            .into(),
                    );
                }
                let planar: Vec<f32> = buffer
                    .channels
                    .iter()
                    .flat_map(|channel| channel.iter().copied())
                    .collect();
                JsFuture::from(
                    encode_audio(
                        id,
                        &planar,
                        count as u32,
                        (audio_start as f64 / sample_rate as f64 * 1_000_000.0).round(),
                    )
                    .map_err(error)?,
                )
                .await
                .map_err(error)?;
                audio_start += count as i64;
            }
        }
        if index + 1 == total || last_progress_at.elapsed().as_millis() >= 100 {
            progress(Some((index + 1) as f64 / total as f64 * 100.0), "rendering");
            last_progress_at = web_time::Instant::now();
        }
        // Let browser codec callbacks and editor events run between rendered frames.
        crate::fs::sleep(0).await;
    }
    progress(None, "finalizing");
    crate::fs::sleep(0).await;
    let output = JsFuture::from(finish_encoder(id).map_err(error)?)
        .await
        .map_err(error)?;
    mux(
        output,
        width,
        height,
        sample_rate,
        channels,
        duration.seconds(),
    )
}

fn mux(
    output: JsValue,
    width: u32,
    height: u32,
    sample_rate: u32,
    channels: u32,
    duration: f64,
) -> Result<Vec<u8>, String> {
    let io = |error: filmcraft_isobmff::Error| error.to_string();
    let description = js_sys::Uint8Array::new(&field(&output, "description")).to_vec();
    let config = AvcConfig::parse(&description).map_err(io)?;
    let mut options = WriterOptions::new(Brand::Mp4);
    options.movie_timescale = 1_000_000;
    let mut writer = Mp4Writer::new(std::io::Cursor::new(Vec::new()), options).map_err(io)?;
    let mut video_track = TrackConfig::new(
        SampleEntry::avc(config, width as u16, height as u16),
        1_000_000,
    );
    video_track.edits = vec![Edit {
        segment_duration: (duration * 1_000_000.0).round() as u64,
        media_time: 0,
        media_rate: 0x0001_0000,
    }];
    let video = writer.add_track(video_track).map_err(io)?;
    let packets = js_sys::Array::from(&field(&output, "video"));
    let mut previous = -1.0;
    for packet in packets.iter() {
        let timestamp = field(&packet, "timestamp")
            .as_f64()
            .ok_or("The browser returned a video packet without a timestamp.")?;
        if timestamp < previous {
            return Err("The browser returned reordered video frames. MP4 saving requires realtime encoding on this platform.".into());
        }
        previous = timestamp;
        let bytes = js_sys::Uint8Array::new(&field(&packet, "data")).to_vec();
        let length = field(&packet, "duration").as_f64().unwrap_or(0.0);
        if length < 1.0 {
            return Err("The browser returned a video packet without a duration.".into());
        }
        writer
            .write_sample(
                video,
                WriteSample {
                    data: &bytes,
                    duration: length.round() as u32,
                    composition_offset: 0,
                    is_sync: field(&packet, "key").as_bool().unwrap_or(false),
                },
            )
            .map_err(io)?;
    }
    if channels > 0 {
        let packets = js_sys::Array::from(&field(&output, "audio"));
        let priming = field(&output, "audioPriming")
            .as_f64()
            .ok_or("The browser did not verify audio synchronization.")?;
        if !priming.is_finite() || !(0.0..=8192.0).contains(&priming) {
            return Err("Invalid audio synchronization offset.".into());
        }
        let description = js_sys::Uint8Array::new(&field(&output, "audioDescription")).to_vec();
        let mut config = TrackConfig::new(
            SampleEntry::aac(description, channels, sample_rate),
            sample_rate,
        );
        config.edits = vec![Edit {
            segment_duration: (duration * 1_000_000.0).round() as u64,
            media_time: priming as i64,
            media_rate: 0x0001_0000,
        }];
        let audio = writer.add_track(config).map_err(io)?;
        for packet in packets.iter() {
            let bytes = js_sys::Uint8Array::new(&field(&packet, "data")).to_vec();
            let samples = (field(&packet, "duration")
                .as_f64()
                .unwrap_or(1024.0 * 1_000_000.0 / sample_rate as f64)
                * sample_rate as f64
                / 1_000_000.0)
                .round() as u32;
            writer
                .write_sample(
                    audio,
                    WriteSample {
                        data: &bytes,
                        duration: samples.max(1),
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .map_err(io)?;
        }
    }
    let bytes =
        add_aac_roll_groups(&writer.finish_faststart().map_err(io)?.into_inner()).map_err(error)?;
    if bytes.len() > 200 * 1024 * 1024 {
        return Err("The exported video exceeds 200 MiB.".into());
    }
    Ok(bytes)
}
