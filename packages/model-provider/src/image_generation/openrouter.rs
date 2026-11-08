//! Build image requests from the selected model's published endpoint capabilities.

use std::collections::BTreeMap;

use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const MAX_ENDPOINTS: usize = 64;
const MAX_DIMENSION: u32 = 2048;
const RASTER_FORMATS: &[&str] = &["png", "jpeg", "webp"];

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Capability {
    Enum { values: Vec<String> },
    Range { min: u64, max: u64 },
    Boolean,
}

impl Capability {
    fn accepts(&self, value: &Value) -> bool {
        match self {
            Self::Enum { values } => value
                .as_str()
                .is_some_and(|value| values.iter().any(|supported| supported == value)),
            Self::Range { min, max } => value
                .as_u64()
                .is_some_and(|value| (*min..=*max).contains(&value)),
            Self::Boolean => true,
        }
    }

    fn values(&self) -> Option<&[String]> {
        match self {
            Self::Enum { values } => Some(values),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Endpoint {
    #[serde(default)]
    provider_tag: Option<String>,
    #[serde(default)]
    provider_slug: Option<String>,
    supported_parameters: BTreeMap<String, Capability>,
}

/// Capabilities are per endpoint: combining a model's union of options can produce
/// a request that no endpoint accepts.
#[derive(Clone, Debug, Deserialize)]
pub struct ImageCapabilities {
    id: String,
    endpoints: Vec<Endpoint>,
}

impl ImageCapabilities {
    pub fn from_endpoints(value: &Value) -> Result<Self> {
        let caps: Self = serde_json::from_value(value.clone())
            .map_err(|_| anyhow::anyhow!("Invalid OpenRouter image capability response"))?;
        validate_model(&caps.id)?;
        ensure!(
            !caps.endpoints.is_empty() && caps.endpoints.len() <= MAX_ENDPOINTS,
            "OpenRouter image capabilities contain no usable endpoint list"
        );
        for endpoint in &caps.endpoints {
            ensure!(
                endpoint.supported_parameters.len() <= 64
                    && endpoint.provider_tag.as_deref().is_none_or(valid_provider)
                    && endpoint.provider_slug.as_deref().is_none_or(valid_provider),
                "Invalid OpenRouter image endpoint capabilities"
            );
            for capability in endpoint.supported_parameters.values() {
                ensure!(
                    match capability {
                        Capability::Enum { values } =>
                            !values.is_empty()
                                && values.len() <= 128
                                && values
                                    .iter()
                                    .all(|value| !value.is_empty() && value.len() <= 128),
                        Capability::Range { min, max } => min <= max,
                        Capability::Boolean => true,
                    },
                    "Invalid OpenRouter image capability descriptor"
                );
            }
        }
        Ok(caps)
    }
}

fn validate_model(model: &str) -> Result<()> {
    let parts: Vec<_> = model.split('/').collect();
    ensure!(
        model.len() <= 512
            && parts.len() == 2
            && parts.iter().all(|part| {
                !part.is_empty()
                    && !matches!(*part, "." | "..")
                    && part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                    })
            }),
        "Invalid OpenRouter image model ID"
    );
    Ok(())
}

fn valid_provider(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= 128
        && provider.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':')
        })
}

/// Resolve discovery on the configured API origin without accepting a model as a URL.
pub fn capabilities_url(base: &str, model: &str) -> Result<String> {
    validate_model(model)?;
    let mut url = reqwest::Url::parse(base)
        .map_err(|_| anyhow::anyhow!("Invalid OpenRouter image API base"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "OpenRouter image API base must be HTTP or HTTPS without credentials, query, or fragment"
    );
    let prefix = if url.path().trim_matches('/').is_empty() {
        "/api/v1"
    } else {
        url.path().trim_end_matches('/')
    };
    url.set_path(&format!("{prefix}/images/models/{model}/endpoints"));
    Ok(url.into())
}

#[derive(Default)]
struct Routing {
    only: Option<Vec<String>>,
    ignore: Vec<String>,
    allow_fallbacks: Option<bool>,
}

impl Routing {
    fn parse(value: Option<&Value>) -> Result<Self> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Invalid image provider routing"))?;
        ensure!(
            object
                .keys()
                .all(|key| matches!(key.as_str(), "only" | "ignore" | "allow_fallbacks")),
            "Image capability routing supports only provider.only, provider.ignore, and provider.allow_fallbacks"
        );
        fn list(value: Option<&Value>) -> Result<Option<Vec<String>>> {
            let Some(value) = value else {
                return Ok(None);
            };
            let values = value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid image provider routing list"))?;
            ensure!(
                values.len() <= MAX_ENDPOINTS,
                "Image provider routing list is too large"
            );
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .filter(|value| valid_provider(value))
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow::anyhow!("Invalid image provider routing entry"))
                })
                .collect::<Result<Vec<_>>>()
                .map(Some)
        }
        let allow_fallbacks = object
            .get("allow_fallbacks")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| anyhow::anyhow!("Invalid image provider fallback setting"))
            })
            .transpose()?;
        Ok(Self {
            only: list(object.get("only"))?,
            ignore: list(object.get("ignore"))?.unwrap_or_default(),
            allow_fallbacks,
        })
    }

    fn allows(&self, endpoint: &Endpoint) -> bool {
        let matches = |providers: &[String]| {
            providers.iter().any(|provider| {
                endpoint.provider_tag.as_ref() == Some(provider)
                    || endpoint.provider_slug.as_ref() == Some(provider)
            })
        };
        self.only.as_ref().is_none_or(|only| matches(only)) && !matches(&self.ignore)
    }
}

/// Normalize before generation. A failed generation is never replayed to probe options.
pub fn prepare_request(model: &str, fields: Value, caps: &ImageCapabilities) -> Result<Value> {
    validate_model(model)?;
    ensure!(
        model == caps.id,
        "OpenRouter image capabilities name a different model"
    );
    let fields = fields
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Image request must be an object"))?;
    ensure!(
        fields.keys().all(|key| matches!(
            key.as_str(),
            "model"
                | "prompt"
                | "n"
                | "stream"
                | "size"
                | "resolution"
                | "aspect_ratio"
                | "quality"
                | "output_format"
                | "background"
                | "seed"
                | "negative_prompt"
                | "provider"
                | "input_references"
                | "output_compression"
        )),
        "Image request contains an unsupported option"
    );
    ensure!(
        fields
            .get("model")
            .is_none_or(|value| value.as_str() == Some(model)),
        "Image request names a different model"
    );
    ensure!(
        fields
            .get("prompt")
            .and_then(Value::as_str)
            .is_some_and(|prompt| !prompt.trim().is_empty()),
        "Image generation needs a prompt"
    );
    ensure!(
        fields.get("stream").is_none_or(|value| value == false),
        "Image generation requires a buffered response"
    );
    let routing = Routing::parse(fields.get("provider"))?;
    let endpoints: Vec<_> = caps
        .endpoints
        .iter()
        .filter(|endpoint| routing.allows(endpoint))
        .collect();
    ensure!(
        !endpoints.is_empty(),
        "No image endpoint matches the requested provider routing"
    );

    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    for endpoint in &endpoints {
        match prepare_endpoint(model, fields, endpoint) {
            Ok((body, score)) => candidates.push((*endpoint, body, score)),
            Err(error) => errors.push(error),
        }
    }
    if candidates.is_empty() {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| anyhow::anyhow!("No compatible image endpoint is available")));
    }
    candidates.sort_by(|left, right| left.2.total_cmp(&right.2));
    for (_, preferred, _) in &candidates {
        let compatible: Vec<_> = candidates
            .iter()
            .filter(|(_, candidate, _)| candidate == preferred)
            .collect();
        let mut tags: Vec<_> = compatible
            .iter()
            .filter_map(|(endpoint, _, _)| endpoint.provider_tag.clone())
            .filter(|tag| {
                // A provider tag can cover multiple endpoint records. Each must accept this body.
                endpoints
                    .iter()
                    .filter(|endpoint| endpoint.provider_tag.as_ref() == Some(tag))
                    .all(|endpoint| {
                        compatible
                            .iter()
                            .any(|entry| std::ptr::eq(entry.0, *endpoint))
                    })
            })
            .collect();
        tags.sort();
        tags.dedup();
        let mut body = preferred.clone();
        if !tags.is_empty() {
            let mut provider = json!({"only": tags});
            if let Some(allow) = routing.allow_fallbacks {
                provider["allow_fallbacks"] = json!(allow);
            }
            body.insert("provider".into(), provider);
            return Ok(Value::Object(body));
        } else if compatible.len() == endpoints.len()
            && routing.only.is_none()
            && routing.ignore.is_empty()
        {
            if let Some(allow) = routing.allow_fallbacks {
                body.insert("provider".into(), json!({"allow_fallbacks": allow}));
            }
            return Ok(Value::Object(body));
        }
    }
    bail!("Compatible image endpoints cannot be selected with provider routing")
}

fn string<'a>(fields: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>> {
    fields
        .get(key)
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Image {key} must be a string"))
        })
        .transpose()
}

fn dimensions(size: &str) -> Option<(u32, u32)> {
    let (width, height) = size.split_once('x')?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    ((256..=MAX_DIMENSION).contains(&width) && (256..=MAX_DIMENSION).contains(&height))
        .then_some((width, height))
}

fn ratio(value: &str) -> Option<f64> {
    let (width, height) = value.split_once(':')?;
    let (width, height) = (width.parse::<f64>().ok()?, height.parse::<f64>().ok()?);
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some(width / height)
}

fn exact_ratio(width: u32, height: u32) -> String {
    let (mut a, mut b) = (width, height);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    format!("{}:{}", width / a, height / a)
}

fn tier_pixels(tier: &str) -> Option<u32> {
    match tier {
        "512" => Some(512),
        "768" => Some(768),
        "1K" => Some(1024),
        "1.5K" => Some(1536),
        "2K" => Some(2048),
        "4K" => Some(4096),
        _ => None,
    }
}

fn prepare_endpoint(
    model: &str,
    fields: &Map<String, Value>,
    endpoint: &Endpoint,
) -> Result<(Map<String, Value>, f64)> {
    let caps = &endpoint.supported_parameters;
    let mut body = Map::from_iter([
        ("model".into(), json!(model)),
        ("prompt".into(), fields["prompt"].clone()),
        ("stream".into(), json!(false)),
    ]);
    let mut score = 0.0;
    let count = fields
        .get("n")
        .map(|value| {
            value
                .as_u64()
                .filter(|count| (1..=10).contains(count))
                .ok_or_else(|| anyhow::anyhow!("Image count must be between 1 and 10"))
        })
        .transpose()?
        .unwrap_or(1);
    if let Some(capability) = caps.get("n") {
        ensure!(
            capability.accepts(&json!(count)),
            "The selected image endpoint does not support the requested image count"
        );
        body.insert("n".into(), json!(count));
    } else {
        ensure!(
            count == 1,
            "The selected image endpoint supports one image per request"
        );
    }

    let references = fields
        .get("input_references")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Image references must be an array"))
        })
        .transpose()?;
    let reference_count = references.map_or(0, Vec::len) as u64;
    if let Some(capability) = caps.get("input_references") {
        ensure!(
            capability.accepts(&json!(reference_count)),
            "The selected image endpoint requires a different number of reference images"
        );
    } else {
        ensure!(
            reference_count == 0,
            "The selected image endpoint does not support reference images"
        );
    }
    if reference_count > 0 {
        body.insert(
            "input_references".into(),
            fields["input_references"].clone(),
        );
    }

    let size = string(fields, "size")?.filter(|value| *value != "auto");
    let pixels = size.and_then(dimensions);
    let explicit_ratio = string(fields, "aspect_ratio")?.filter(|value| *value != "auto");
    let resolution_field = string(fields, "resolution")?;
    ensure!(
        pixels.is_none() || resolution_field.is_none(),
        "Specify pixel size or resolution, not both"
    );
    if let (Some(size), Some(resolution)) = (size, resolution_field) {
        ensure!(size == resolution, "Image size and resolution disagree");
    }
    let explicit_resolution = resolution_field.or(size.filter(|_| pixels.is_none()));
    if let Some(value) = explicit_ratio {
        ensure!(ratio(value).is_some(), "Invalid image aspect ratio");
    }
    if let (Some((width, height)), Some(value)) = (pixels, explicit_ratio) {
        ensure!(
            (ratio(value).unwrap() - f64::from(width) / f64::from(height)).abs() < 1e-8,
            "Image size and aspect ratio disagree"
        );
    }
    if size.is_some_and(|value| value.contains('x')) && pixels.is_none() {
        bail!("Image size must have dimensions between 256 and 2048");
    }
    if let (Some((width, height)), Some(capability)) = (pixels, caps.get("size")) {
        let value = json!(format!("{width}x{height}"));
        ensure!(
            capability.accepts(&value),
            "The selected image endpoint does not support this pixel size"
        );
        body.insert("size".into(), value);
    } else {
        let wanted_ratio = explicit_ratio
            .map(str::to_owned)
            .or_else(|| pixels.map(|(w, h)| exact_ratio(w, h)));
        if let Some(wanted) = wanted_ratio {
            match caps.get("aspect_ratio") {
                Some(capability) if capability.accepts(&json!(wanted)) => {
                    body.insert("aspect_ratio".into(), json!(wanted));
                }
                Some(capability) if explicit_ratio.is_none() => {
                    let target = ratio(&wanted).unwrap();
                    let mut supported: Vec<_> = capability
                        .values()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|value| ratio(value).map(|candidate| (value, candidate)))
                        .filter(|(_, candidate)| {
                            candidate.total_cmp(&1.0) == target.total_cmp(&1.0)
                        })
                        .collect();
                    supported.sort_by(|a, b| {
                        (a.1 / target)
                            .ln()
                            .abs()
                            .total_cmp(&(b.1 / target).ln().abs())
                    });
                    let Some((selected, value)) = supported.first() else {
                        bail!(
                            "The selected image endpoint does not support the requested image orientation"
                        );
                    };
                    score += (value / target).ln().abs() * 100.0;
                    body.insert("aspect_ratio".into(), json!(selected));
                }
                None if explicit_ratio.is_none() => {
                    score += 1000.0;
                }
                _ => {
                    bail!("The selected image endpoint does not support the requested aspect ratio")
                }
            }
        } else if string(fields, "aspect_ratio")? == Some("auto")
            && caps
                .get("aspect_ratio")
                .is_some_and(|cap| cap.accepts(&json!("auto")))
        {
            body.insert("aspect_ratio".into(), json!("auto"));
        }
        if let Some(resolution) = explicit_resolution {
            ensure!(
                tier_pixels(resolution).is_some_and(|value| value <= MAX_DIMENSION),
                "Image resolution must not exceed 2K"
            );
            ensure!(
                caps.get("resolution")
                    .is_some_and(|cap| cap.accepts(&json!(resolution))),
                "The selected image endpoint does not support the requested resolution"
            );
            body.insert("resolution".into(), json!(resolution));
        } else if let Some((width, height)) = pixels {
            if let Some(capability) = caps.get("resolution") {
                let target = width.max(height);
                let mut supported: Vec<_> = capability
                    .values()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|tier| {
                        tier_pixels(tier)
                            .filter(|value| *value <= MAX_DIMENSION)
                            .map(|value| (tier, value))
                    })
                    .collect();
                supported.sort_by_key(|(_, value)| (value.abs_diff(target), *value));
                let Some((selected, value)) = supported.first() else {
                    bail!(
                        "The selected image endpoint has no supported resolution within the 2K limit"
                    );
                };
                score += f64::from(value.abs_diff(target)) / f64::from(target);
                body.insert("resolution".into(), json!(selected));
            } else {
                // Prefer an endpoint that can honor the resolution as well as the shape.
                score += 10.0;
            }
        }
    }

    for name in [
        "quality",
        "output_format",
        "background",
        "seed",
        "negative_prompt",
        "output_compression",
    ] {
        let Some(value) = fields.get(name).filter(|value| !value.is_null()) else {
            continue;
        };
        if matches!(name, "quality" | "background")
            && value == "auto"
            && !caps.get(name).is_some_and(|cap| cap.accepts(value))
        {
            continue;
        }
        if name == "negative_prompt" && value.as_str().is_some_and(|value| value.trim().is_empty())
        {
            continue;
        }
        ensure!(
            match name {
                "seed" => value
                    .as_u64()
                    .is_some_and(|value| value <= u64::from(u32::MAX)),
                "output_compression" => value.as_u64().is_some_and(|value| value <= 100),
                _ => value.as_str().is_some(),
            },
            "Invalid image {name}"
        );
        ensure!(
            caps.get(name).is_some_and(|cap| cap.accepts(value)),
            "The selected image endpoint does not support the requested {name}"
        );
        if name == "output_format" {
            ensure!(
                value
                    .as_str()
                    .is_some_and(|value| RASTER_FORMATS.contains(&value)),
                "Image generation supports PNG, JPEG, and WebP output"
            );
        }
        body.insert(name.into(), value.clone());
    }
    if !body.contains_key("output_format") {
        if let Some(formats) = caps.get("output_format").and_then(Capability::values) {
            let raster = RASTER_FORMATS
                .iter()
                .find(|format| formats.iter().any(|value| value == **format));
            ensure!(
                raster.is_some(),
                "The selected image endpoint does not produce a supported raster format"
            );
            if formats
                .iter()
                .any(|format| !RASTER_FORMATS.contains(&format.as_str()))
            {
                body.insert("output_format".into(), json!(raster.unwrap()));
            }
        }
    }
    if body
        .get("background")
        .is_some_and(|value| value == "transparent")
    {
        ensure!(
            body.get("output_format")
                .is_none_or(|value| value != "jpeg"),
            "Transparent image output requires PNG or WebP"
        );
        if !body.contains_key("output_format") {
            if let Some(formats) = caps.get("output_format").and_then(Capability::values) {
                let format = ["png", "webp"]
                    .into_iter()
                    .find(|format| formats.iter().any(|value| value == format));
                ensure!(
                    format.is_some(),
                    "The selected image endpoint does not support a transparent output format"
                );
                body.insert("output_format".into(), json!(format.unwrap()));
            }
        }
    }
    Ok((body, score))
}

#[cfg(test)]
#[path = "openrouter/tests.rs"]
mod tests;
