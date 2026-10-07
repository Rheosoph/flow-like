use anyhow::{Result, ensure};
use std::f64::consts::PI;

use crate::embedding::interface::AudioInput;

const SAMPLE_RATE: usize = 16_000;
const FRAME: usize = 320;
const HOP: usize = 160;
const FFT: usize = 512;
pub(super) const MEL_BINS: usize = 128;

pub(super) struct AudioFeatures {
    pub values: Vec<f32>,
    pub mask: Vec<bool>,
    pub tokens: usize,
}

pub(super) fn token_count(audio: &AudioInput) -> Result<usize> {
    ensure!(
        audio.channels > 0 && audio.sample_rate > 0,
        "Audio needs a positive sample rate and channel count"
    );
    ensure!(
        !audio.samples.is_empty() && audio.samples.len().is_multiple_of(audio.channels as usize),
        "Audio samples must contain complete interleaved frames"
    );
    let source_frames = audio.samples.len() / audio.channels as usize;
    let ratio = SAMPLE_RATE as f64 / audio.sample_rate as f64;
    let samples = (source_frames as f64 * ratio).ceil() as usize;
    ensure!(
        samples > FRAME / 2,
        "Audio must contain more than 10 ms of samples"
    );
    Ok(((samples - 1) / HOP).div_ceil(4))
}

pub(super) fn preprocess(audio: &AudioInput) -> Result<AudioFeatures> {
    ensure!(
        audio.channels > 0 && audio.sample_rate > 0,
        "Audio needs a positive sample rate and channel count"
    );
    ensure!(
        !audio.samples.is_empty() && audio.samples.len().is_multiple_of(audio.channels as usize),
        "Audio samples must contain complete interleaved frames"
    );
    ensure!(
        audio.samples.iter().all(|value| value.is_finite()),
        "Audio contains non-finite samples"
    );
    let mono: Vec<f32> = audio
        .samples
        .chunks_exact(audio.channels as usize)
        .map(|frame| frame.iter().map(|v| *v as f64).sum::<f64>() as f32 / audio.channels as f32)
        .collect();
    let waveform = resample(&mono, audio.sample_rate as usize);
    ensure!(
        waveform.len() > FRAME / 2,
        "Audio must contain more than 10 ms of samples"
    );
    // The reference pads raw samples to a multiple of 128 before semicausal STFT.
    let padded_samples = waveform.len().div_ceil(128) * 128;
    let frames = (padded_samples + FRAME / 2 - (FRAME + 1)) / HOP + 1;
    let window: Vec<f32> = (0..FRAME)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / FRAME as f64).cos()) as f32)
        .collect();
    let filters = mel_filters();
    let mut values = vec![0.0; frames * MEL_BINS];
    let mut mask = Vec::with_capacity(frames);
    for frame in 0..frames {
        let valid = frame * HOP + FRAME / 2 < waveform.len();
        mask.push(valid);
        if !valid {
            continue;
        }
        let mut real = [0.0; FFT];
        let mut imaginary = [0.0; FFT];
        for sample in 0..FRAME {
            let position = frame * HOP + sample;
            if position >= FRAME / 2 && position - FRAME / 2 < waveform.len() {
                // NumPy multiplies the float32 waveform and window before its float64 FFT.
                real[sample] = (waveform[position - FRAME / 2] * window[sample]) as f64;
            }
        }
        fft(&mut real, &mut imaginary);
        let magnitudes: Vec<f64> = (0..=FFT / 2)
            .map(|bin| real[bin].hypot(imaginary[bin]))
            .collect();
        for mel in 0..MEL_BINS {
            let magnitude = (0..=FFT / 2)
                .map(|bin| magnitudes[bin] * filters[bin * MEL_BINS + mel])
                .sum::<f64>();
            values[frame * MEL_BINS + mel] = (magnitude + 0.001).ln() as f32;
        }
    }
    // The encoder applies two padded stride-two convolutions and selects valid rows.
    let tokens = mask.iter().step_by(4).filter(|valid| **valid).count();
    ensure!(tokens > 0, "Audio did not produce any valid encoder tokens");
    Ok(AudioFeatures {
        values,
        mask,
        tokens,
    })
}

fn mel_filters() -> Vec<f64> {
    let mel_max = 2595.0 * (1.0_f64 + 8000.0 / 700.0).log10();
    let frequencies: Vec<f64> = (0..MEL_BINS + 2)
        .map(|i| 700.0 * (10_f64.powf((i as f64 * mel_max / (MEL_BINS + 1) as f64) / 2595.0) - 1.0))
        .collect();
    let mut filters = vec![0.0; (FFT / 2 + 1) * MEL_BINS];
    for bin in 0..=FFT / 2 {
        let frequency = bin as f64 * SAMPLE_RATE as f64 / FFT as f64;
        for mel in 0..MEL_BINS {
            let ascending =
                (frequency - frequencies[mel]) / (frequencies[mel + 1] - frequencies[mel]);
            let descending =
                (frequencies[mel + 2] - frequency) / (frequencies[mel + 2] - frequencies[mel + 1]);
            filters[bin * MEL_BINS + mel] = ascending.min(descending).max(0.0);
        }
    }
    filters
}

fn fft(real: &mut [f64; FFT], imaginary: &mut [f64; FFT]) {
    for index in 0..FFT {
        let reversed = index.reverse_bits() >> (usize::BITS - FFT.ilog2());
        if index < reversed {
            real.swap(index, reversed);
        }
    }
    let mut size = 2;
    while size <= FFT {
        let half = size / 2;
        for start in (0..FFT).step_by(size) {
            for offset in 0..half {
                let angle = -2.0 * PI * offset as f64 / size as f64;
                let (sin, cos) = angle.sin_cos();
                let left = start + offset;
                let right = left + half;
                let r = cos * real[right] - sin * imaginary[right];
                let i = sin * real[right] + cos * imaginary[right];
                real[right] = real[left] - r;
                imaginary[right] = imaginary[left] - i;
                real[left] += r;
                imaginary[left] += i;
            }
        }
        size *= 2;
    }
}

// Windowed-sinc interpolation also low-pass filters when downsampling.
fn resample(samples: &[f32], source_rate: usize) -> Vec<f32> {
    if source_rate == SAMPLE_RATE {
        return samples.to_vec();
    }
    let ratio = SAMPLE_RATE as f64 / source_rate as f64;
    let length = (samples.len() as f64 * ratio).ceil() as usize;
    let cutoff = ratio.min(1.0) * 0.95;
    let radius = 32.0 / cutoff;
    (0..length)
        .map(|index| {
            let center = index as f64 / ratio;
            let start = (center - radius).ceil().max(0.0) as usize;
            let end = ((center + radius).floor() as usize + 1).min(samples.len());
            let mut sum = 0.0;
            let mut weights = 0.0;
            for (position, sample) in samples.iter().enumerate().take(end).skip(start) {
                let distance = position as f64 - center;
                let phase = PI * distance * cutoff;
                let sinc = if phase.abs() < 1e-10 {
                    1.0
                } else {
                    phase.sin() / phase
                };
                let weight = sinc * 0.5 * (1.0 + (PI * distance / radius).cos());
                sum += *sample as f64 * weight;
                weights += weight;
            }
            (sum / weights) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn silence_has_reference_log_floor_and_dynamic_token_count() {
        let features = preprocess(&AudioInput {
            samples: Arc::from(vec![0.0; 16_000]),
            sample_rate: 16_000,
            channels: 1,
        })
        .unwrap();
        assert_eq!(features.mask.len(), 99);
        assert_eq!(features.tokens, 25);
        assert!(
            features
                .values
                .iter()
                .all(|value| (*value - 0.001_f32.ln()).abs() < 1e-6)
        );
    }

    #[test]
    fn preflight_audio_count_matches_padding_and_subsampling_boundaries() {
        for samples in [
            161, 319, 320, 321, 639, 640, 641, 1023, 1024, 1025, 15_999, 16_001,
        ] {
            let audio = AudioInput {
                samples: Arc::from(vec![0.0; samples]),
                sample_rate: 16_000,
                channels: 1,
            };
            assert_eq!(
                token_count(&audio).unwrap(),
                preprocess(&audio).unwrap().tokens,
                "{samples} samples"
            );
        }
    }

    #[test]
    fn downsampling_filters_out_high_frequency_aliases() {
        let samples: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * PI * 12_000.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let output = resample(&samples, 48_000);
        let energy = output[100..output.len() - 100]
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            / (output.len() - 200) as f32;
        assert!(energy < 1e-5, "aliased energy {energy}");
    }
}
