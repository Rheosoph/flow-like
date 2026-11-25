mod aiff;
mod raw;
mod source;
pub use raw::{RawPcm, aiff_pcm, wav_pcm};
mod wav;

use crate::pcm::{PcmKind, deinterleave};
use crate::{AudioBuffer, AudioError, AudioInfo, Result};
use source::Src;

pub const DECODE_EXTENSIONS: &[&str] = &["wav", "wave", "bwf", "rf64", "aif", "aiff", "aifc"];
const MAX_SAMPLES: u64 = (256 * 1024 * 1024) / 4;

fn unsupported() -> AudioError {
    AudioError::Unsupported(
        "compressed audio must be decoded by the browser before importing".into(),
    )
}

fn layout(bytes: &[u8]) -> Result<(AudioInfo, PcmKind, u64, u64)> {
    let source = Src::Slice(bytes);
    if wav::is_wav(bytes) {
        return match wav::parse(&source)? {
            wav::WavParse::Native(layout) => {
                Ok((layout.info, layout.kind, layout.data_start, layout.data_len))
            }
            wav::WavParse::Foreign { .. } => Err(unsupported()),
        };
    }
    if aiff::is_aiff(bytes) {
        return match aiff::parse(&source)? {
            aiff::AiffParse::Native(layout) => {
                Ok((layout.info, layout.kind, layout.data_start, layout.data_len))
            }
            aiff::AiffParse::Foreign => Err(unsupported()),
        };
    }
    Err(unsupported())
}

pub fn probe(bytes: &[u8], _extension: Option<&str>) -> Result<AudioInfo> {
    layout(bytes).map(|(info, _, _, _)| info)
}

pub fn decode(bytes: &[u8], _extension: Option<&str>) -> Result<(AudioInfo, AudioBuffer)> {
    let (mut info, kind, start, length) = layout(bytes)?;
    let samples = info.frames.saturating_mul(u64::from(info.channels));
    if samples > MAX_SAMPLES {
        return Err(AudioError::TooLarge(
            "decoded audio exceeds the editor's 256 MiB PCM limit".into(),
        ));
    }
    let start = usize::try_from(start)
        .map_err(|_| AudioError::Malformed("PCM offset exceeds address space".into()))?;
    let length = usize::try_from(length)
        .map_err(|_| AudioError::Malformed("PCM length exceeds address space".into()))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| AudioError::Malformed("PCM range overflows".into()))?;
    let data = bytes
        .get(start..end)
        .ok_or_else(|| AudioError::Malformed("PCM data is truncated".into()))?;
    let channels = deinterleave(data, kind, usize::from(info.channels), info.frames)?;
    info.frames = channels.first().map_or(0, |channel| channel.len() as u64);
    let buffer = AudioBuffer {
        sample_rate: info.sample_rate,
        channels,
    };
    Ok((info, buffer))
}
