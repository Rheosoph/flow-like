use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{SigningKey, StandaloneRelease, sign_standalone_release};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 4 {
        return Err("Usage: sign-release SIGNING_KEY_FILE MANIFEST_JSON OUTPUT_JWS".into());
    }
    let key_path = Path::new(&args[1]);
    let metadata = std::fs::symlink_metadata(key_path)?;
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
    let mut encoded = Vec::new();
    std::fs::File::open(key_path)?
        .take(129)
        .read_to_end(&mut encoded)?;
    let mut decoded = URL_SAFE_NO_PAD.decode(std::str::from_utf8(&encoded)?.trim())?;
    let mut seed: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| "Release signing key must decode to 32 bytes")?;
    let key = SigningKey::from_bytes(&seed);
    seed.fill(0);
    decoded.fill(0);
    encoded.fill(0);
    let mut bytes = Vec::new();
    std::fs::File::open(&args[2])?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 {
        return Err("Release manifest is too large".into());
    }
    let manifest: StandaloneRelease = serde_json::from_slice(&bytes)?;
    let signed = sign_standalone_release(&manifest, &key)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])?;
    output.write_all(signed.as_bytes())?;
    output.sync_all()?;
    println!(
        "{}",
        serde_json::json!({"signer_fingerprint":key.public_key().thumbprint()?,"public_key":key.public_key(),"release_version":manifest.release_version,"sequence":manifest.sequence})
    );
    Ok(())
}
