//! Reads the few GGUF header values that size a model's KV cache and context. Every length is
//! bounded and all other values are skipped without being parsed.

use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

const MAGIC: &[u8; 4] = b"GGUF";
const MAX_KEYS: u64 = 1 << 20;
const MAX_KEY_BYTES: u64 = 64 * 1024;
const MAX_STRING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARRAY_ITEMS: u64 = 1 << 30;
const MAX_HEADER_BYTES: u64 = 1 << 30;
const MAX_DEPTH: u8 = 2;
const WANTED: [&str; 7] = [
    "block_count",
    "context_length",
    "embedding_length",
    "attention.head_count",
    "attention.head_count_kv",
    "attention.key_length",
    "attention.value_length",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GgufFacts {
    pub architecture: Option<String>,
    pub block_count: Option<u64>,
    pub context_length: Option<u64>,
    pub embedding_length: Option<u64>,
    pub head_count: Option<u64>,
    /// Summed over layers when the model lists one count per layer.
    pub head_count_kv_total: Option<u64>,
    pub key_length: Option<u64>,
    pub value_length: Option<u64>,
}

impl GgufFacts {
    /// K plus V elements one token takes in the cache across all layers.
    pub fn kv_elements_per_token(&self) -> Option<u64> {
        let head_dim = self
            .embedding_length
            .zip(self.head_count)
            .filter(|(_, heads)| *heads > 0)
            .map(|(width, heads)| width / heads);
        let key = self.key_length.or(head_dim)?;
        let value = self.value_length.or(head_dim)?;
        let heads = match self.head_count_kv_total {
            Some(heads) => heads,
            None => self.head_count?.checked_mul(self.block_count?)?,
        };
        heads.checked_mul(key.checked_add(value)?)
    }
}

enum Number {
    Unsigned(u64),
    Signed(i64),
    Float,
}

struct Reader<R> {
    inner: R,
    consumed: u64,
}

impl<R: Read> Reader<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut buffer = [0u8; N];
        self.inner.read_exact(&mut buffer)?;
        self.advance(N as u64)?;
        Ok(buffer)
    }

    fn advance(&mut self, bytes: u64) -> Result<()> {
        self.consumed = self.consumed.saturating_add(bytes);
        ensure!(
            self.consumed <= MAX_HEADER_BYTES,
            "GGUF header exceeds {MAX_HEADER_BYTES} bytes"
        );
        Ok(())
    }

    fn skip(&mut self, bytes: u64) -> Result<()> {
        self.advance(bytes)?;
        let copied = std::io::copy(&mut (&mut self.inner).take(bytes), &mut std::io::sink())?;
        ensure!(copied == bytes, "GGUF header ends early");
        Ok(())
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }

    fn string(&mut self, limit: u64) -> Result<String> {
        let length = self.u64()?;
        ensure!(
            length <= limit,
            "GGUF string of {length} bytes exceeds {limit}"
        );
        self.advance(length)?;
        let mut bytes = vec![0u8; length as usize];
        self.inner.read_exact(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn skip_string(&mut self) -> Result<()> {
        let length = self.u64()?;
        ensure!(
            length <= MAX_STRING_BYTES,
            "GGUF string of {length} bytes is too long"
        );
        self.skip(length)
    }

    /// Integers widen to 64 bits, sign-extended for the signed types.
    fn number(&mut self, kind: u32) -> Result<Number> {
        let size = Self::fixed_size(kind)
            .with_context(|| format!("GGUF value type {kind} is not a number"))?;
        let mut bytes = [0u8; 8];
        self.inner.read_exact(&mut bytes[..size as usize])?;
        self.advance(size)?;
        if matches!(kind, 6 | 12) {
            return Ok(Number::Float);
        }
        let unsigned = u64::from_le_bytes(bytes);
        if matches!(kind, 1 | 3 | 5 | 11) {
            let shift = 64 - 8 * size as u32;
            return Ok(Number::Signed(((unsigned << shift) as i64) >> shift));
        }
        Ok(Number::Unsigned(unsigned))
    }

    fn fixed_size(kind: u32) -> Option<u64> {
        match kind {
            0 | 1 | 7 => Some(1),
            2 | 3 => Some(2),
            4..=6 => Some(4),
            10..=12 => Some(8),
            _ => None,
        }
    }

    fn skip_value(&mut self, kind: u32, depth: u8) -> Result<()> {
        if let Some(size) = Self::fixed_size(kind) {
            return self.skip(size);
        }
        match kind {
            8 => self.skip_string(),
            9 => {
                ensure!(depth < MAX_DEPTH, "GGUF arrays nest too deeply");
                let item = self.u32()?;
                let length = self.u64()?;
                ensure!(
                    length <= MAX_ARRAY_ITEMS,
                    "GGUF array of {length} items is too long"
                );
                match Self::fixed_size(item) {
                    Some(size) => self.skip(
                        size.checked_mul(length)
                            .context("GGUF array size overflows")?,
                    ),
                    None => (0..length).try_for_each(|_| self.skip_value(item, depth + 1)),
                }
            }
            other => bail!("Unknown GGUF value type {other}"),
        }
    }

    /// A number, or the sum of an array of numbers.
    fn total(&mut self, kind: u32) -> Result<Option<Value>> {
        if kind == 9 {
            return self.per_layer_sum();
        }
        if Self::fixed_size(kind).is_none() {
            self.skip_value(kind, 0)?;
            return Ok(None);
        }
        Ok(self.number(kind)?.whole().map(Value::Scalar))
    }

    fn per_layer_sum(&mut self) -> Result<Option<Value>> {
        let item = self.u32()?;
        let length = self.u64()?;
        ensure!(
            length <= 1 << 16,
            "GGUF per-layer array of {length} items is too long"
        );
        ensure!(
            Self::fixed_size(item).is_some(),
            "GGUF per-layer array holds no numbers"
        );
        let mut sum = Some(0u64);
        for _ in 0..length {
            let value = self.number(item)?.whole();
            sum = sum.zip(value).map(|(sum, value)| sum.saturating_add(value));
        }
        Ok(sum.map(Value::PerLayer))
    }
}

impl Number {
    /// The value when it is a whole number of zero or more.
    fn whole(self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(value),
            Self::Signed(value) => u64::try_from(value).ok(),
            Self::Float => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Value {
    Scalar(u64),
    /// The sum of one value per layer.
    PerLayer(u64),
}

impl Value {
    fn scalar(self) -> Option<u64> {
        match self {
            Self::Scalar(value) => Some(value),
            Self::PerLayer(_) => None,
        }
    }
}

pub fn read(path: &Path) -> Result<GgufFacts> {
    let file = File::open(path).with_context(|| format!("Open GGUF {}", path.display()))?;
    parse(BufReader::new(file))
        .with_context(|| format!("Read the GGUF header of {}", path.display()))
}

fn parse(inner: impl Read) -> Result<GgufFacts> {
    let mut reader = Reader { inner, consumed: 0 };
    let keys = reader.header()?;
    let mut header = Header::default();
    for _ in 0..keys {
        reader.entry(&mut header)?;
    }
    Ok(facts(header.architecture, &header.values))
}

#[derive(Default)]
struct Header {
    architecture: Option<String>,
    values: HashMap<String, Value>,
}

impl<R: Read> Reader<R> {
    /// The key count, once magic, version and tensor count check out.
    fn header(&mut self) -> Result<u64> {
        ensure!(&self.bytes::<4>()? == MAGIC, "Not a GGUF file");
        let version = self.u32()?;
        ensure!(
            matches!(version, 2 | 3),
            "Unsupported GGUF version {version}"
        );
        self.u64()?;
        let keys = self.u64()?;
        ensure!(keys <= MAX_KEYS, "GGUF lists {keys} keys");
        Ok(keys)
    }

    fn entry(&mut self, header: &mut Header) -> Result<()> {
        let key = self.string(MAX_KEY_BYTES)?;
        let kind = self.u32()?;
        match (key.as_str(), kind) {
            ("general.architecture", 8) => {
                header.architecture = Some(self.string(256)?);
                Ok(())
            }
            _ if wanted(&key) => self.keep(header, key, kind),
            _ => self.skip_value(kind, 0),
        }
    }

    fn keep(&mut self, header: &mut Header, key: String, kind: u32) -> Result<()> {
        if let Some(value) = self.total(kind)? {
            header.values.insert(key, value);
        }
        Ok(())
    }
}

fn wanted(key: &str) -> bool {
    key.split_once('.')
        .is_some_and(|(_, suffix)| WANTED.contains(&suffix))
}

fn facts(architecture: Option<String>, values: &HashMap<String, Value>) -> GgufFacts {
    let entry = |suffix: &str| {
        architecture
            .as_ref()
            .and_then(|architecture| values.get(&format!("{architecture}.{suffix}")))
            .copied()
    };
    let value = |suffix: &str| entry(suffix).and_then(Value::scalar);
    let block_count = value("block_count");
    let head_count_kv_total = match entry("attention.head_count_kv") {
        Some(Value::PerLayer(sum)) => Some(sum),
        Some(Value::Scalar(heads)) => block_count.and_then(|blocks| heads.checked_mul(blocks)),
        None => None,
    };
    GgufFacts {
        head_count_kv_total,
        block_count,
        context_length: value("context_length"),
        embedding_length: value("embedding_length"),
        head_count: value("attention.head_count"),
        key_length: value("attention.key_length"),
        value_length: value("attention.value_length"),
        architecture,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Writer(pub Vec<u8>);

    impl Writer {
        pub fn new(keys: u64) -> Self {
            let mut bytes = MAGIC.to_vec();
            bytes.extend(3u32.to_le_bytes());
            bytes.extend(0u64.to_le_bytes());
            bytes.extend(keys.to_le_bytes());
            Self(bytes)
        }

        fn key(&mut self, key: &str, kind: u32) {
            self.0.extend((key.len() as u64).to_le_bytes());
            self.0.extend(key.as_bytes());
            self.0.extend(kind.to_le_bytes());
        }

        pub fn string(mut self, key: &str, value: &str) -> Self {
            self.key(key, 8);
            self.0.extend((value.len() as u64).to_le_bytes());
            self.0.extend(value.as_bytes());
            self
        }

        pub fn u32(mut self, key: &str, value: u32) -> Self {
            self.key(key, 4);
            self.0.extend(value.to_le_bytes());
            self
        }

        pub fn strings(mut self, key: &str, values: &[&str]) -> Self {
            self.key(key, 9);
            self.0.extend(8u32.to_le_bytes());
            self.0.extend((values.len() as u64).to_le_bytes());
            for value in values {
                self.0.extend((value.len() as u64).to_le_bytes());
                self.0.extend(value.as_bytes());
            }
            self
        }
    }

    /// A Llama-3-8B-shaped header: 32 layers, 32 heads, 8 KV heads, 4096 wide.
    pub(crate) fn llama_header() -> Vec<u8> {
        Writer::new(7)
            .string("general.architecture", "llama")
            .strings("tokenizer.ggml.tokens", &["a", "b", "<s>"])
            .u32("llama.block_count", 32)
            .u32("llama.context_length", 8192)
            .u32("llama.embedding_length", 4096)
            .u32("llama.attention.head_count", 32)
            .u32("llama.attention.head_count_kv", 8)
            .0
    }

    #[test]
    fn a_header_sizes_the_kv_cache() -> Result<()> {
        let facts = parse(llama_header().as_slice())?;
        assert_eq!(facts.architecture.as_deref(), Some("llama"));
        assert_eq!(facts.context_length, Some(8192));
        assert_eq!(facts.kv_elements_per_token(), Some(32 * 8 * 256));
        Ok(())
    }

    #[test]
    fn hostile_headers_are_refused() {
        assert!(parse(&b"GGML"[..]).is_err());
        let mut long = Writer::new(1).0;
        long.extend(u64::MAX.to_le_bytes());
        assert!(parse(long.as_slice()).is_err());
        let truncated = &llama_header()[..40];
        assert!(parse(truncated).is_err());
    }
}
