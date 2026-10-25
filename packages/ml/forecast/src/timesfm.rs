//! Native inference for Google's Apache-2.0 TimesFM 2.5 checkpoints.
//!
//! Uses the official quickstart's normalization, continuous quantile head, flip
//! averaging, positivity inference and quantile crossing repair. History is
//! limited to 15,360 steps, reserving 1,024 steps of the 16,384-step context
//! budget for forecasting. Each series has one target; XReg covariates are not
//! supported. Missing observations follow upstream interpolation semantics.
// Architecture and decoding adapted from google-research/timesfm, Apache-2.0.
// Copyright 2025 Google LLC.

use crate::{
    Error, ForecastCapabilities, ForecastFamily, ForecastRequest, ForecastResult, PointStatistic,
    Result, SeriesForecast, types::validate_workspace, weights::Weights,
};
use burn::tensor::{
    Bool, Device, Tensor, TensorData,
    activation::{silu, softmax},
};
use serde::Deserialize;
use std::path::Path;

const QUANTILES: [f32; 9] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
const CHANNELS: usize = 10;
const MEDIAN: usize = 5;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    architectures: Vec<String>,
    model_type: String,
    context_length: usize,
    head_dim: usize,
    hidden_size: usize,
    horizon_length: usize,
    intermediate_size: usize,
    num_attention_heads: usize,
    num_hidden_layers: usize,
    patch_length: usize,
    quantile_horizon_length: usize,
    quantiles: Vec<f32>,
    rms_norm_eps: f64,
    #[serde(default, rename = "torch_compile")]
    _torch_compile: bool,
}

impl Config {
    fn validate(&self) -> Result<()> {
        if self.model_type != "timesfm"
            || self.architectures != ["TimesFmModelForPrediction"]
            || self.quantiles != QUANTILES
            || self.context_length == 0
            || self.context_length > 16_384
            || self.patch_length == 0
            || self.horizon_length == 0
            || self.quantile_horizon_length == 0
            || self.quantile_horizon_length > 1024
            || self.quantile_horizon_length >= self.context_length
            || self.hidden_size == 0
            || self.intermediate_size == 0
            || self.num_attention_heads == 0
            || self.head_dim == 0
            || self.head_dim % 2 != 0
            || self.num_attention_heads.checked_mul(self.head_dim) != Some(self.hidden_size)
            || self.num_hidden_layers == 0
            || self.num_hidden_layers > 64
            || !self.horizon_length.is_multiple_of(self.patch_length)
            || !self
                .quantile_horizon_length
                .is_multiple_of(self.horizon_length)
            || !self.context_length.is_multiple_of(self.patch_length)
            || !self.rms_norm_eps.is_finite()
            || self.rms_norm_eps <= 0.0
            || !(self.rms_norm_eps as f32).is_finite()
            || self.rms_norm_eps as f32 == 0.0
        {
            return Err(invalid(
                "Unsupported TimesFM 2.5 architecture configuration",
            ));
        }
        for (shape, label) in [
            (vec![2, self.patch_length, self.hidden_size], "tokenizer"),
            (vec![3, self.hidden_size, self.hidden_size], "QKV weights"),
            (
                vec![self.hidden_size, self.intermediate_size],
                "feedforward weights",
            ),
            (
                vec![self.quantile_horizon_length, CHANNELS, self.hidden_size],
                "quantile head",
            ),
        ] {
            validate_workspace(&shape, &format!("TimesFM {label}"))?;
        }
        let patches = self.context_length / self.patch_length;
        validate_workspace(
            &[self.num_attention_heads, patches, patches],
            "TimesFM attention",
        )?;
        validate_workspace(
            &[self.num_hidden_layers, 2, patches, self.hidden_size],
            "TimesFM KV cache",
        )?;
        validate_workspace(&[patches, 3, self.hidden_size], "TimesFM QKV projection")?;
        validate_workspace(&[patches, self.intermediate_size], "TimesFM feedforward")?;
        Ok(())
    }

    fn max_context(&self) -> usize {
        self.context_length - self.quantile_horizon_length
    }

    fn specs(&self) -> Vec<(String, Vec<usize>)> {
        let mut specs = Vec::new();
        let d = self.hidden_size;
        let mut linear = |name: String, input: usize, output: usize, bias: bool| {
            specs.push((format!("{name}.weight"), vec![output, input]));
            if bias {
                specs.push((format!("{name}.bias"), vec![output]));
            }
        };
        for (name, input, output, bias) in [
            ("tokenizer", 2 * self.patch_length, d, true),
            (
                "output_projection_point",
                d,
                self.horizon_length * CHANNELS,
                false,
            ),
            (
                "output_projection_quantiles",
                d,
                self.quantile_horizon_length * CHANNELS,
                false,
            ),
        ] {
            linear(format!("{name}.hidden_layer"), input, d, bias);
            linear(format!("{name}.output_layer"), d, output, bias);
            linear(format!("{name}.residual_layer"), input, output, bias);
        }
        for layer in 0..self.num_hidden_layers {
            let p = format!("stacked_xf.{layer}");
            linear(format!("{p}.attn.qkv_proj"), d, 3 * d, false);
            linear(format!("{p}.attn.out"), d, d, false);
            linear(format!("{p}.ff0"), d, self.intermediate_size, false);
            linear(format!("{p}.ff1"), self.intermediate_size, d, false);
        }
        for layer in 0..self.num_hidden_layers {
            let p = format!("stacked_xf.{layer}");
            for name in ["pre_attn_ln", "post_attn_ln", "pre_ff_ln", "post_ff_ln"] {
                specs.push((format!("{p}.{name}.scale"), vec![d]));
            }
            for name in [
                "query_ln.scale",
                "key_ln.scale",
                "per_dim_scale.per_dim_scale",
            ] {
                specs.push((format!("{p}.attn.{name}"), vec![self.head_dim]));
            }
        }
        specs
    }
}

struct Linear {
    weight: Tensor<2>,
    bias: Option<Tensor<1>>,
}
impl Linear {
    fn load(w: &Weights, name: &str, bias: bool, device: &Device) -> Result<Self> {
        Ok(Self {
            weight: w
                .tensor::<2>(&format!("{name}.weight"), device)?
                .transpose(),
            bias: if bias {
                Some(w.tensor(&format!("{name}.bias"), device)?)
            } else {
                None
            },
        })
    }
    fn forward(&self, x: Tensor<3>) -> Tensor<3> {
        let output = x.matmul(self.weight.clone().unsqueeze());
        match &self.bias {
            Some(bias) => output + bias.clone().unsqueeze::<3>(),
            None => output,
        }
    }
}

struct Residual {
    hidden: Linear,
    output: Linear,
    residual: Linear,
}
impl Residual {
    fn load(w: &Weights, name: &str, bias: bool, device: &Device) -> Result<Self> {
        Ok(Self {
            hidden: Linear::load(w, &format!("{name}.hidden_layer"), bias, device)?,
            output: Linear::load(w, &format!("{name}.output_layer"), bias, device)?,
            residual: Linear::load(w, &format!("{name}.residual_layer"), bias, device)?,
        })
    }
    fn forward(&self, x: Tensor<3>) -> Tensor<3> {
        self.output.forward(silu(self.hidden.forward(x.clone()))) + self.residual.forward(x)
    }
}

struct Norm {
    scale: Tensor<1>,
    epsilon: f64,
}
impl Norm {
    fn load(w: &Weights, name: &str, epsilon: f64, device: &Device) -> Result<Self> {
        Ok(Self {
            scale: w.tensor(&format!("{name}.scale"), device)?,
            epsilon,
        })
    }
    fn forward<const D: usize>(&self, x: Tensor<D>) -> Tensor<D> {
        let inv = (x.clone().powf_scalar(2.0).mean_dim(D - 1) + self.epsilon)
            .sqrt()
            .recip();
        x * inv * self.scale.clone().unsqueeze::<D>()
    }
}

struct Cache {
    key: Tensor<4>,
    value: Tensor<4>,
    masked: usize,
}

struct Layer {
    pre_attn: Norm,
    post_attn: Norm,
    pre_ff: Norm,
    post_ff: Norm,
    query_norm: Norm,
    key_norm: Norm,
    qkv: Linear,
    out: Linear,
    ff0: Linear,
    ff1: Linear,
    query_scale: Tensor<1>,
    heads: usize,
    head_dim: usize,
}
impl Layer {
    fn load(w: &Weights, index: usize, c: &Config, device: &Device) -> Result<Self> {
        let p = format!("stacked_xf.{index}");
        let scales: Vec<f32> = w
            .values(&format!("{p}.attn.per_dim_scale.per_dim_scale"))?
            .into_iter()
            .map(|x| {
                // Stable softplus matches PyTorch's thresholded expression.
                let softplus = if x > 20.0 { x } else { x.exp().ln_1p() };
                softplus * (std::f64::consts::LOG2_E / (c.head_dim as f64).sqrt()) as f32
            })
            .collect();
        Ok(Self {
            pre_attn: Norm::load(w, &format!("{p}.pre_attn_ln"), c.rms_norm_eps, device)?,
            post_attn: Norm::load(w, &format!("{p}.post_attn_ln"), c.rms_norm_eps, device)?,
            pre_ff: Norm::load(w, &format!("{p}.pre_ff_ln"), c.rms_norm_eps, device)?,
            post_ff: Norm::load(w, &format!("{p}.post_ff_ln"), c.rms_norm_eps, device)?,
            query_norm: Norm::load(w, &format!("{p}.attn.query_ln"), c.rms_norm_eps, device)?,
            key_norm: Norm::load(w, &format!("{p}.attn.key_ln"), c.rms_norm_eps, device)?,
            qkv: Linear::load(w, &format!("{p}.attn.qkv_proj"), false, device)?,
            out: Linear::load(w, &format!("{p}.attn.out"), false, device)?,
            ff0: Linear::load(w, &format!("{p}.ff0"), false, device)?,
            ff1: Linear::load(w, &format!("{p}.ff1"), false, device)?,
            query_scale: Tensor::from_data(TensorData::new(scales, [c.head_dim]), device),
            heads: c.num_attention_heads,
            head_dim: c.head_dim,
        })
    }

    fn forward(&self, input: Tensor<3>, masked: usize, cache: &mut Option<Cache>) -> Tensor<3> {
        let [batch, count, width] = input.dims();
        let previous = cache.as_ref().map_or(0, |c| c.key.dims()[1]);
        let all_masked = masked + cache.as_ref().map_or(0, |c| c.masked);
        let qkv = self.qkv.forward(self.pre_attn.forward(input.clone()));
        let q = qkv.clone().slice([0..batch, 0..count, 0..width]).reshape([
            batch,
            count,
            self.heads,
            self.head_dim,
        ]);
        let k = qkv
            .clone()
            .slice([0..batch, 0..count, width..2 * width])
            .reshape([batch, count, self.heads, self.head_dim]);
        let v = qkv
            .slice([0..batch, 0..count, 2 * width..3 * width])
            .reshape([batch, count, self.heads, self.head_dim]);
        let q = self.query_norm.forward(rotate(q, previous, all_masked))
            * self.query_scale.clone().unsqueeze::<4>();
        let k = self.key_norm.forward(rotate(k, previous, all_masked));
        let (k, v) = match cache.take() {
            Some(old) => (
                Tensor::cat(vec![old.key, k], 1),
                Tensor::cat(vec![old.value, v], 1),
            ),
            None => (k, v),
        };
        let keys = k.dims()[1];
        let blocked: Vec<bool> = (0..count)
            .flat_map(|q| (0..keys).map(move |k| k < all_masked || k > previous + q))
            .collect();
        let mask = Tensor::<4, Bool>::from_data(
            TensorData::new(blocked, [1, 1, count, keys]),
            &input.device(),
        );
        let scores = q.swap_dims(1, 2).matmul(k.clone().permute([0, 2, 3, 1]));
        // Fully masked padding queries have zero attention, as in torch SDPA.
        let attention =
            softmax(scores.mask_fill(mask.clone(), -f32::MAX / 2.0), 3).mask_fill(mask, 0.0);
        let attended = attention
            .matmul(v.clone().swap_dims(1, 2))
            .swap_dims(1, 2)
            .reshape([batch, count, width]);
        *cache = Some(Cache {
            key: k,
            value: v,
            masked: all_masked,
        });
        let x = input + self.post_attn.forward(self.out.forward(attended));
        x.clone()
            + self.post_ff.forward(
                self.ff1
                    .forward(silu(self.ff0.forward(self.pre_ff.forward(x)))),
            )
    }
}

fn rotate(x: Tensor<4>, offset: usize, masked: usize) -> Tensor<4> {
    let [batch, count, heads, width] = x.dims();
    let half = width / 2;
    let mut cos = Vec::with_capacity(count * half);
    let mut sin = Vec::with_capacity(count * half);
    for position in 0..count {
        for dim in 0..half {
            let angle = (offset as f64 + position as f64 - masked as f64)
                / 10_000_f64.powf(2.0 * dim as f64 / width as f64);
            cos.push(angle.cos() as f32);
            sin.push(angle.sin() as f32);
        }
    }
    let cos = Tensor::<4>::from_data(TensorData::new(cos, [1, count, 1, half]), &x.device());
    let sin = Tensor::<4>::from_data(TensorData::new(sin, [1, count, 1, half]), &x.device());
    let a = x.clone().slice([0..batch, 0..count, 0..heads, 0..half]);
    let b = x.slice([0..batch, 0..count, 0..heads, half..width]);
    Tensor::cat(
        vec![
            a.clone() * cos.clone() - b.clone() * sin.clone(),
            b * cos + a * sin,
        ],
        3,
    )
}

#[derive(Default, Clone, Copy)]
struct Stats {
    count: f64,
    mean: f64,
    sigma: f64,
}
impl Stats {
    fn update(&mut self, values: &[f32], missing: &[bool]) {
        let n = missing.iter().filter(|&&m| !m).count() as f64;
        let mean = values
            .iter()
            .zip(missing)
            .filter(|(_, m)| !**m)
            .map(|(x, _)| *x as f64)
            .sum::<f64>()
            / n.max(1.0);
        let variance = values
            .iter()
            .zip(missing)
            .filter(|(_, m)| !**m)
            .map(|(x, _)| (*x as f64 - mean).powi(2))
            .sum::<f64>()
            / n.max(1.0);
        let count = self.count + n;
        let next_mean = (self.count * self.mean + n * mean) / count.max(1.0);
        let next_var = (self.count * self.sigma.powi(2)
            + n * variance
            + self.count * (self.mean - next_mean).powi(2)
            + n * (mean - next_mean).powi(2))
            / count.max(1.0);
        *self = Self {
            count,
            mean: next_mean,
            sigma: next_var.max(0.0).sqrt(),
        };
    }
}

struct Prepared {
    values: Vec<f32>,
    missing: Vec<bool>,
    mean: f64,
    sigma: f64,
    positive: bool,
}

fn prepare(values: &[f32], observed: Option<&[bool]>, c: &Config) -> Result<Prepared> {
    let observed = |index: usize| observed.is_none_or(|mask| mask[index]);
    let first = (0..values.len())
        .find(|&i| observed(i))
        .ok_or_else(|| invalid("TimesFM needs an observed target value"))?;
    let mut filled = values[first..].to_vec();
    let mut previous = first;
    for next in first + 1..values.len() {
        if !observed(next) {
            continue;
        }
        for i in previous + 1..next {
            let weight = (i - previous) as f64 / (next - previous) as f64;
            filled[i - first] =
                ((1.0 - weight) * values[previous] as f64 + weight * values[next] as f64) as f32;
        }
        previous = next;
    }
    filled[previous + 1 - first..].fill(values[previous]);
    let start = filled.len().saturating_sub(c.max_context());
    let filled = &filled[start..];
    let padding = (c.patch_length - filled.len() % c.patch_length) % c.patch_length;
    let mut values = vec![0.0; padding];
    values.extend_from_slice(filled);
    let positive = values.iter().all(|x| *x >= 0.0);
    // Upstream pads to its configured maximum before outer normalization.
    // Include those zeros in the statistics without sending fully masked
    // patches through the transformer. This also preserves the 1e-6 fallback.
    let context = c.max_context();
    let virtual_padding = context - values.len();
    let mean = values.iter().map(|x| *x as f64).sum::<f64>() / context as f64;
    let sigma = (values
        .iter()
        .map(|x| (*x as f64 - mean).powi(2))
        .sum::<f64>()
        + virtual_padding as f64 * mean.powi(2))
        / (context - 1).max(1) as f64;
    let sigma = sigma.sqrt();
    let divisor = if sigma < 1e-6 { 1.0 } else { sigma };
    values
        .iter_mut()
        .for_each(|x| *x = ((*x as f64 - mean) / divisor) as f32);
    let missing = (0..values.len()).map(|i| i < padding).collect();
    Ok(Prepared {
        values,
        missing,
        mean,
        sigma,
        positive,
    })
}

/// TimesFM 2.5's native point and continuous-quantile checkpoint heads.
pub struct TimesFm {
    config: Config,
    tokenizer: Residual,
    layers: Vec<Layer>,
    point_head: Residual,
    quantile_head: Residual,
    device: Device,
}

impl TimesFm {
    pub fn load(directory: &Path, device: &Device) -> Result<Self> {
        let config: Config =
            serde_json::from_slice(&std::fs::read(directory.join("config.json"))?)?;
        config.validate()?;
        let weights = Weights::open(directory)?;
        let specs = config.specs();
        // Validate the complete checkpoint before transferring any tensors.
        for (name, shape) in &specs {
            if weights.shape(name)? != *shape {
                return Err(invalid(format!(
                    "TimesFM tensor {name} must have shape {shape:?}"
                )));
            }
        }
        if weights.names().count() != specs.len() {
            return Err(invalid("Unexpected tensors in TimesFM 2.5 checkpoint"));
        }
        Ok(Self {
            tokenizer: Residual::load(&weights, "tokenizer", true, device)?,
            layers: (0..config.num_hidden_layers)
                .map(|i| Layer::load(&weights, i, &config, device))
                .collect::<Result<_>>()?,
            point_head: Residual::load(&weights, "output_projection_point", false, device)?,
            quantile_head: Residual::load(&weights, "output_projection_quantiles", false, device)?,
            config,
            device: device.clone(),
        })
    }

    pub fn capabilities(&self) -> ForecastCapabilities {
        ForecastCapabilities {
            family: ForecastFamily::Timesfm25,
            max_context: self.config.max_context(),
            max_horizon: Some(self.config.quantile_horizon_length),
            native_horizon: self.config.horizon_length,
            quantile_levels: QUANTILES.to_vec(),
            joint_multivariate: false,
            past_covariates: false,
            known_covariates: false,
        }
    }

    fn embed(
        &self,
        values: &[f32],
        missing: &[bool],
        stats: &mut Stats,
        caches: &mut [Option<Cache>],
    ) -> (Tensor<3>, Stats) {
        let patch = self.config.patch_length;
        let count = values.len() / patch;
        let mut tokens = Vec::with_capacity(values.len() * 2);
        for (values, missing) in values.chunks_exact(patch).zip(missing.chunks_exact(patch)) {
            stats.update(values, missing);
            let divisor = if stats.sigma < 1e-6 { 1.0 } else { stats.sigma };
            tokens.extend(values.iter().zip(missing).map(|(x, m)| {
                if *m {
                    0.0
                } else {
                    ((*x as f64 - stats.mean) / divisor) as f32
                }
            }));
            tokens.extend(missing.iter().map(|m| if *m { 1.0 } else { 0.0 }));
        }
        let masked = missing.chunks_exact(patch).filter(|m| m[patch - 1]).count();
        let mut x = self.tokenizer.forward(Tensor::from_data(
            TensorData::new(tokens, [1, count, 2 * patch]),
            &self.device,
        ));
        for (layer, cache) in self.layers.iter().zip(caches) {
            x = layer.forward(x, masked, cache);
        }
        // Only the final embedding is consumed by either forecast head.
        (
            x.slice([0..1, count - 1..count, 0..self.config.hidden_size]),
            *stats,
        )
    }

    fn head(
        &self,
        head: &Residual,
        embedding: Tensor<3>,
        stats: Stats,
    ) -> Result<Vec<[f32; CHANNELS]>> {
        let values = head
            .forward(embedding)
            .into_data()
            .try_to_vec::<f32>()
            .map_err(|e| Error::Tensor(format!("TimesFM output transfer: {e:?}")))?;
        let values: Vec<f32> = values
            .into_iter()
            .map(|x| (x as f64 * stats.sigma + stats.mean) as f32)
            .collect();
        if values.iter().any(|x| !x.is_finite()) {
            return Err(invalid("TimesFM produced non-finite forecast values"));
        }
        Ok(values
            .chunks_exact(CHANNELS)
            .map(|x| x.try_into().unwrap())
            .collect())
    }

    fn decode(
        &self,
        values: &[f32],
        missing: &[bool],
        horizon: usize,
    ) -> Result<(Vec<[f32; CHANNELS]>, Vec<[f32; CHANNELS]>)> {
        let mut caches: Vec<Option<Cache>> = (0..self.layers.len()).map(|_| None).collect();
        let mut stats = Stats::default();
        let (embedding, current) = self.embed(values, missing, &mut stats, &mut caches);
        let spread = self.head(&self.quantile_head, embedding.clone(), current)?;
        let mut output = self.head(&self.point_head, embedding, current)?;
        while output.len() < horizon {
            let latest = &output[output.len() - self.config.horizon_length..];
            let next: Vec<f32> = latest.iter().map(|row| row[MEDIAN]).collect();
            let (embedding, current) =
                self.embed(&next, &vec![false; next.len()], &mut stats, &mut caches);
            output.extend(self.head(&self.point_head, embedding, current)?);
        }
        output.truncate(horizon);
        Ok((output, spread))
    }

    pub fn forecast(&self, request: &ForecastRequest) -> Result<ForecastResult> {
        request.validate_for_quantiles(QUANTILES.len())?;
        if request.horizon > self.config.quantile_horizon_length {
            return Err(invalid(format!(
                "TimesFM continuous quantiles support at most {} forecast steps",
                self.config.quantile_horizon_length
            )));
        }
        if request
            .series
            .iter()
            .any(|s| !s.past_covariates.is_empty() || !s.known_covariates.is_empty())
        {
            return Err(invalid(
                "TimesFM native inference does not support covariates; upstream XReg is a separate regression workflow",
            ));
        }
        if request.series.iter().any(|s| s.targets.len() != 1) {
            return Err(invalid(
                "TimesFM requires one target per series; use separate series for independent targets",
            ));
        }
        let levels = if request.quantile_levels.is_empty() {
            QUANTILES.to_vec()
        } else {
            request.quantile_levels.clone()
        };
        let selected = levels
            .iter()
            .map(|q| {
                QUANTILES
                    .iter()
                    .position(|native| native == q)
                    .map(|i| i + 1)
                    .ok_or_else(|| {
                        invalid("TimesFM supports quantiles 0.1 through 0.9 in increments of 0.1")
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut results = Vec::with_capacity(request.series.len());
        for series in &request.series {
            let mut points = Vec::with_capacity(series.targets.len());
            let mut quantiles = Vec::with_capacity(series.targets.len());
            for (index, target) in series.targets.iter().enumerate() {
                let prepared = prepare(
                    target,
                    series.observed_mask.as_ref().map(|m| m[index].as_slice()),
                    &self.config,
                )?;
                let (forward, spread) =
                    self.decode(&prepared.values, &prepared.missing, request.horizon)?;
                let negated: Vec<f32> = prepared.values.iter().map(|x| -*x).collect();
                let (reverse, reverse_spread) =
                    self.decode(&negated, &prepared.missing, request.horizon)?;
                let rows = combine(&forward, &spread, &reverse, &reverse_spread, &prepared);
                points.push(rows.iter().map(|row| row[MEDIAN]).collect());
                quantiles.push(
                    rows.iter()
                        .map(|row| selected.iter().map(|i| row[*i]).collect())
                        .collect(),
                );
            }
            results.push(SeriesForecast {
                id: series.id.clone(),
                point: points,
                quantiles,
            });
        }
        let result = ForecastResult {
            series: results,
            quantile_levels: levels,
            point_statistic: PointStatistic::Median,
        };
        result.validate(request)?;
        Ok(result)
    }
}

fn combine(
    forward: &[[f32; CHANNELS]],
    spread: &[[f32; CHANNELS]],
    reverse: &[[f32; CHANNELS]],
    reverse_spread: &[[f32; CHANNELS]],
    prepared: &Prepared,
) -> Vec<[f32; CHANNELS]> {
    forward
        .iter()
        .enumerate()
        .map(|(t, values)| {
            let mut row = [0.0; CHANNELS];
            for i in 0..CHANNELS {
                let flipped = if i == 0 { 0 } else { CHANNELS - i };
                row[i] = (values[i] - reverse[t][flipped]) * 0.5;
            }
            let spread_median = (spread[t][MEDIAN] - reverse_spread[t][MEDIAN]) * 0.5;
            for i in [1, 2, 3, 4, 6, 7, 8, 9] {
                row[i] = (spread[t][i] - reverse_spread[t][CHANNELS - i]) * 0.5 - spread_median
                    + row[MEDIAN];
            }
            for i in (1..MEDIAN).rev() {
                row[i] = row[i].min(row[i + 1]);
            }
            for i in MEDIAN + 1..CHANNELS {
                row[i] = row[i].max(row[i - 1]);
            }
            for x in &mut row {
                *x = (*x as f64 * prepared.sigma + prepared.mean) as f32;
                if prepared.positive {
                    *x = x.max(0.0);
                }
            }
            row
        })
        .collect()
}

#[cfg(all(test, feature = "cpu"))]
mod tests;
