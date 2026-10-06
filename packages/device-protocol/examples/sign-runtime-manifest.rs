use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    MAX_COMPACT_JWS_BYTES, RuntimeManifest, SigningKey, sign_runtime_manifest,
};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 4 {
        return Err(
            "Usage: sign-runtime-manifest SIGNING_KEY_FILE RUNTIME_MANIFEST_JSON OUTPUT_JWS".into(),
        );
    }
    let key = signing_key(Path::new(&args[1]))?;
    let manifest = read_manifest(&args[2])?;
    let signed = sign_runtime_manifest(&manifest, &key)
        .map_err(|error| format!("Cannot sign runtime manifest {}: {error}", args[2]))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])?;
    output.write_all(signed.as_bytes())?;
    output.sync_all()?;
    println!("{}", report(&key, &manifest, signed.len())?);
    Ok(())
}

fn read_manifest(path: &str) -> Result<RuntimeManifest, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_COMPACT_JWS_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_COMPACT_JWS_BYTES {
        return Err(
            format!("Runtime manifest {path} exceeds {MAX_COMPACT_JWS_BYTES} bytes").into(),
        );
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn check_key_file(path: &Path) -> Result<(), Box<dyn Error>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 128 {
        return Err("Release signing key must be a small regular private file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Release signing key file must have mode 0600".into());
        }
    }
    Ok(())
}

fn signing_key(path: &Path) -> Result<SigningKey, Box<dyn Error>> {
    check_key_file(path)?;
    let mut encoded = Vec::new();
    File::open(path)?.take(129).read_to_end(&mut encoded)?;
    let mut decoded = URL_SAFE_NO_PAD.decode(std::str::from_utf8(&encoded)?.trim())?;
    let mut seed: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| "Release signing key must decode to 32 bytes")?;
    let key = SigningKey::from_bytes(&seed);
    seed.fill(0);
    decoded.fill(0);
    encoded.fill(0);
    Ok(key)
}

fn report(
    key: &SigningKey,
    manifest: &RuntimeManifest,
    compact_bytes: usize,
) -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(serde_json::json!({
        "signer_fingerprint": key.public_key().thumbprint()?,
        "public_key": key.public_key(),
        "sequence": manifest.sequence,
        "issued_at": manifest.issued_at,
        "expires_at": manifest.expires_at,
        "lifetime_days": (manifest.expires_at - manifest.issued_at) as f64 / 86_400.0,
        "compact_bytes": compact_bytes,
        "packs": manifest.packs.iter().map(|pack| serde_json::json!({
            "runtime": pack.runtime,
            "build": pack.build,
            "target": pack.target,
            "backend": pack.backend,
            "size": pack.size,
            "sha256": pack.sha256,
            "files": pack.files.len(),
        })).collect::<Vec<_>>(),
    }))
}
