//! Signed runtime packs (llama.cpp, MLX) that a device installs on demand.
//!
//! A pack is a `.tar.gz` holding [`RUNTIME_PACK_LISTING`] plus exactly the regular files of
//! [`RuntimePack::files`]: no directories, links or other entries. Every signed manifest covers
//! the packs of one target, because the whole compact JWS must fit
//! [`MAX_COMPACT_JWS_BYTES`](crate::MAX_COMPACT_JWS_BYTES).

use crate::{
    Ed25519PublicKey, MAX_RELEASE_LIFETIME, ModelBackend, ModelRuntime, ProtocolError,
    ReleaseTarget, Result, SigningKey,
    proof::{sign_pinned, verify_pinned},
    validate_artifact_digest, validate_artifact_relative_path, validate_release_url,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const RUNTIME_MANIFEST_JWS_TYPE: &str = "flow-like-runtime-manifest";
/// The archive's own file list at its root. It is never one of the pack's `files`.
pub const RUNTIME_PACK_LISTING: &str = "pack.json";
/// Libraries a host normally provides (C++ runtime, Vulkan loader). The engine adds this
/// directory to the loader path only when it cannot start without it, as in a scratch
/// container; preferring the host's copies keeps GPU drivers on the C++ runtime they need.
pub const RUNTIME_PACK_FALLBACK_DIR: &str = "fallback";
pub const MAX_RUNTIME_PACKS: usize = 16;
pub const MAX_RUNTIME_PACK_FILES: usize = 64;
pub const MAX_RUNTIME_PACK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_RUNTIME_PACK_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_BUILD_ID_LEN: usize = 64;
const NOT_YET_VALID_TOLERANCE: i64 = 300;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimePackFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimePack {
    pub runtime: ModelRuntime,
    pub build: String,
    pub target: ReleaseTarget,
    pub backend: ModelBackend,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub entrypoint: String,
    /// Sorted by path; the archive holds these files in this order after the listing.
    pub files: Vec<RuntimePackFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub version: u32,
    pub sequence: u64,
    pub issued_at: i64,
    pub expires_at: i64,
    pub packs: Vec<RuntimePack>,
}

impl RuntimeManifest {
    /// The pack to install for a slot. Should a manifest list several builds of one slot,
    /// the first one listed is current.
    pub fn find(
        &self,
        runtime: ModelRuntime,
        target: ReleaseTarget,
        backend: ModelBackend,
    ) -> Option<&RuntimePack> {
        self.packs.iter().find(|pack| {
            pack.runtime == runtime && pack.target == target && pack.backend == backend
        })
    }
}

fn build_id(value: &str) -> bool {
    value.len() <= MAX_BUILD_ID_LEN
        && value
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn supported(runtime: ModelRuntime, target: ReleaseTarget, backend: ModelBackend) -> bool {
    let macos = matches!(
        target,
        ReleaseTarget::MacosAarch64 | ReleaseTarget::MacosX86_64
    );
    match (runtime, backend) {
        (ModelRuntime::Mlx, ModelBackend::Metal) => target == ReleaseTarget::MacosAarch64,
        (ModelRuntime::Mlx, _) => false,
        (ModelRuntime::Llamacpp, ModelBackend::Cpu) => true,
        (ModelRuntime::Llamacpp, ModelBackend::Metal) => macos,
        (ModelRuntime::Llamacpp, ModelBackend::Vulkan | ModelBackend::Cuda) => !macos,
    }
}

fn validate_pack_file(file: &RuntimePackFile) -> Result<()> {
    validate_artifact_relative_path(&file.path)?;
    validate_artifact_digest(&file.sha256)?;
    if !file
        .path
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+' | b'/'))
    {
        return Err(ProtocolError::Invalid("runtime pack file path characters"));
    }
    if file.size == 0 {
        return Err(ProtocolError::Invalid("empty runtime pack file"));
    }
    Ok(())
}

fn validate_pack_files(files: &[RuntimePackFile]) -> Result<()> {
    if !(1..=MAX_RUNTIME_PACK_FILES).contains(&files.len()) {
        return Err(ProtocolError::Invalid("runtime pack file count"));
    }
    if files.windows(2).any(|pair| pair[1].path <= pair[0].path) {
        return Err(ProtocolError::Invalid("runtime pack file order"));
    }
    let mut folded = HashSet::new();
    let mut total = 0u64;
    for file in files {
        validate_pack_file(file)?;
        let lower = file.path.to_ascii_lowercase();
        if lower == RUNTIME_PACK_LISTING || !folded.insert(lower) {
            return Err(ProtocolError::Invalid("runtime pack file name collision"));
        }
        total = total
            .checked_add(file.size)
            .filter(|total| *total <= MAX_RUNTIME_PACK_UNPACKED_BYTES)
            .ok_or(ProtocolError::Invalid("runtime pack unpacked size"))?;
    }
    let nested = |path: &String| {
        path.match_indices('/')
            .any(|(offset, _)| folded.contains(&path[..offset]))
    };
    if folded.iter().any(nested) {
        return Err(ProtocolError::Invalid(
            "runtime pack file/directory collision",
        ));
    }
    Ok(())
}

fn validate_runtime_pack(pack: &RuntimePack) -> Result<()> {
    if !supported(pack.runtime, pack.target, pack.backend) {
        return Err(ProtocolError::Invalid(
            "runtime pack runtime, target and backend combination",
        ));
    }
    if !build_id(&pack.build) {
        return Err(ProtocolError::Invalid("runtime pack build"));
    }
    if pack.size == 0 || pack.size > MAX_RUNTIME_PACK_BYTES {
        return Err(ProtocolError::Invalid("runtime pack archive size"));
    }
    validate_release_url(&pack.url)?;
    validate_artifact_digest(&pack.sha256)?;
    validate_pack_files(&pack.files)?;
    if !pack
        .files
        .iter()
        .any(|file| file.path == pack.entrypoint && file.executable)
    {
        return Err(ProtocolError::Invalid(
            "runtime pack entrypoint must be an executable pack file",
        ));
    }
    Ok(())
}

fn validate_packs(packs: &[RuntimePack]) -> Result<()> {
    let mut slots = HashSet::new();
    for pack in packs {
        validate_runtime_pack(pack)?;
        if !slots.insert((pack.runtime, pack.build.as_str(), pack.target, pack.backend)) {
            return Err(ProtocolError::Invalid(
                "runtime pack listed twice for one build",
            ));
        }
    }
    Ok(())
}

pub fn validate_runtime_manifest(value: &RuntimeManifest) -> Result<()> {
    let lifetime = value.expires_at.checked_sub(value.issued_at).unwrap_or(0);
    if value.version != 1
        || !(1..=MAX_SAFE_INTEGER).contains(&value.sequence)
        || value.issued_at < 0
        || value.expires_at > MAX_SAFE_INTEGER as i64
        || !(1..=MAX_RELEASE_LIFETIME).contains(&lifetime)
        || !(1..=MAX_RUNTIME_PACKS).contains(&value.packs.len())
    {
        return Err(ProtocolError::Invalid("runtime manifest shape"));
    }
    validate_packs(&value.packs)
}

pub fn sign_runtime_manifest(value: &RuntimeManifest, key: &SigningKey) -> Result<String> {
    validate_runtime_manifest(value)?;
    sign_pinned(value, key, RUNTIME_MANIFEST_JWS_TYPE)
}

fn validate_trust(pinned: &[Ed25519PublicKey], minimum_sequence: u64, now: i64) -> Result<()> {
    if pinned.is_empty() || pinned.len() > 8 || minimum_sequence > MAX_SAFE_INTEGER || now < 0 {
        return Err(ProtocolError::Invalid(
            "runtime manifest trust configuration",
        ));
    }
    pinned.iter().try_for_each(Ed25519PublicKey::validate)
}

/// The same release keys sign standalone releases; the JWS type keeps the two documents apart.
/// `minimum_sequence` is the highest runtime manifest sequence this device accepted before.
pub fn verify_runtime_manifest(
    compact: &str,
    pinned: &[Ed25519PublicKey],
    minimum_sequence: u64,
    now: i64,
) -> Result<RuntimeManifest> {
    validate_trust(pinned, minimum_sequence, now)?;
    let value: RuntimeManifest = pinned
        .iter()
        .find_map(|key| verify_pinned(compact, key, RUNTIME_MANIFEST_JWS_TYPE).ok())
        .ok_or(ProtocolError::InvalidSignature)?;
    validate_runtime_manifest(&value)?;
    if value.issued_at > now.saturating_add(NOT_YET_VALID_TOLERANCE) || value.expires_at <= now {
        return Err(ProtocolError::InvalidTime);
    }
    if value.sequence < minimum_sequence {
        return Err(ProtocolError::Invalid("runtime manifest rollback"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MAX_COMPACT_JWS_BYTES, STANDALONE_RELEASE_JWS_TYPE, sign_standalone_release,
        verify_standalone_release,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    const BASE: &str = "https://cdn.example/standalone/releases/7/runtimes";

    fn file(path: &str, executable: bool) -> RuntimePackFile {
        RuntimePackFile {
            path: path.into(),
            size: 4096,
            sha256: "c".repeat(64),
            executable,
        }
    }

    fn wire(value: impl Serialize) -> String {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .into()
    }

    fn pack(backend: ModelBackend) -> RuntimePack {
        RuntimePack {
            runtime: ModelRuntime::Llamacpp,
            build: "b10809".into(),
            target: ReleaseTarget::LinuxX86_64,
            backend,
            url: format!(
                "{BASE}/llamacpp-b10809-x86_64-unknown-linux-gnu-{}.tar.gz",
                wire(backend)
            ),
            size: 16_734_586,
            sha256: "a".repeat(64),
            entrypoint: "llama-server".into(),
            files: vec![
                file("fallback/libstdc++.so.6", false),
                file("libggml-base.so.0", false),
                file("libllama.so.0", false),
                file("llama-server", true),
            ],
        }
    }

    fn manifest() -> RuntimeManifest {
        RuntimeManifest {
            version: 1,
            sequence: 7,
            issued_at: 1_000,
            expires_at: 2_000,
            packs: vec![pack(ModelBackend::Cpu), pack(ModelBackend::Vulkan)],
        }
    }

    fn changed(change: impl FnOnce(&mut RuntimeManifest)) -> RuntimeManifest {
        let mut value = manifest();
        change(&mut value);
        value
    }

    fn rejected(value: &RuntimeManifest) -> bool {
        validate_runtime_manifest(value).is_err()
            && sign_runtime_manifest(value, &SigningKey::generate()).is_err()
    }

    #[test]
    fn signed_manifest_round_trips_with_pinned_keys_time_and_sequence() {
        let key = SigningKey::generate();
        let other = SigningKey::generate();
        let value = manifest();
        let signed = sign_runtime_manifest(&value, &key).unwrap();
        let keys = [other.public_key(), key.public_key()];
        assert_eq!(
            verify_runtime_manifest(&signed, &keys, 7, 1_000).unwrap(),
            value
        );
        assert!(matches!(
            verify_runtime_manifest(&signed, &[other.public_key()], 0, 1_000),
            Err(ProtocolError::InvalidSignature)
        ));
        assert!(verify_runtime_manifest(&signed, &[], 0, 1_000).is_err());
        assert!(verify_runtime_manifest(&signed, &keys, 0, -1).is_err());
        assert!(matches!(
            verify_runtime_manifest(&signed, &keys, 8, 1_000),
            Err(ProtocolError::Invalid("runtime manifest rollback"))
        ));
        for now in [699, 2_000, i64::MAX] {
            assert!(matches!(
                verify_runtime_manifest(&signed, &keys, 0, now),
                Err(ProtocolError::InvalidTime)
            ));
        }
        assert_eq!(
            verify_runtime_manifest(&signed, &keys, 0, 700).unwrap(),
            value
        );
        let found = value
            .find(
                ModelRuntime::Llamacpp,
                ReleaseTarget::LinuxX86_64,
                ModelBackend::Vulkan,
            )
            .unwrap();
        assert_eq!(found, &value.packs[1]);
        assert!(
            value
                .find(
                    ModelRuntime::Mlx,
                    ReleaseTarget::LinuxX86_64,
                    ModelBackend::Vulkan
                )
                .is_none()
        );
    }

    #[test]
    fn tampered_payload_or_signature_is_rejected() {
        let key = SigningKey::generate();
        let keys = [key.public_key()];
        let signed = sign_runtime_manifest(&manifest(), &key).unwrap();
        let segments = signed.split('.').collect::<Vec<_>>();
        let mut swapped = manifest();
        swapped.packs[0].sha256 = "b".repeat(64);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&swapped).unwrap());
        let forged = [segments[0], payload.as_str(), segments[2]].join(".");
        assert!(matches!(
            verify_runtime_manifest(&forged, &keys, 0, 1_000),
            Err(ProtocolError::InvalidSignature)
        ));
        let mut signature = URL_SAFE_NO_PAD.decode(segments[2]).unwrap();
        signature[0] ^= 1;
        let flipped = [segments[0], segments[1], &URL_SAFE_NO_PAD.encode(signature)].join(".");
        assert!(verify_runtime_manifest(&flipped, &keys, 0, 1_000).is_err());
        assert!(verify_runtime_manifest(&signed[1..], &keys, 0, 1_000).is_err());
    }

    #[test]
    fn release_and_runtime_documents_never_stand_in_for_each_other() {
        let key = SigningKey::generate();
        let keys = [key.public_key()];
        let as_release = sign_pinned(&manifest(), &key, STANDALONE_RELEASE_JWS_TYPE).unwrap();
        assert!(matches!(
            verify_runtime_manifest(&as_release, &keys, 0, 1_000),
            Err(ProtocolError::InvalidSignature)
        ));
        let runtime = sign_runtime_manifest(&manifest(), &key).unwrap();
        assert!(verify_standalone_release(&runtime, &keys, 0, 1_000).is_err());
        let release = crate::StandaloneRelease {
            version: 1,
            state_schema_version: 4,
            sequence: 7,
            release_version: "1.2.3".into(),
            issued_at: 1_000,
            expires_at: 2_000,
            artifacts: vec![crate::StandaloneArtifact {
                target: ReleaseTarget::LinuxX86_64,
                url: "https://cdn.example/standalone/releases/7/agent".into(),
                size: 1,
                sha256: "a".repeat(64),
            }],
            container: None,
        };
        let signed_release = sign_standalone_release(&release, &key).unwrap();
        assert!(verify_runtime_manifest(&signed_release, &keys, 0, 1_000).is_err());
    }

    #[test]
    fn lifetime_follows_the_release_cap_at_signing_and_verification() {
        let key = SigningKey::generate();
        let keys = [key.public_key()];
        let longest = changed(|value| value.expires_at = value.issued_at + MAX_RELEASE_LIFETIME);
        let signed = sign_runtime_manifest(&longest, &key).unwrap();
        assert!(verify_runtime_manifest(&signed, &keys, 0, longest.expires_at - 1).is_ok());
        let beyond = changed(|value| value.expires_at = value.issued_at + MAX_RELEASE_LIFETIME + 1);
        assert!(rejected(&beyond));
        let unchecked = sign_pinned(&beyond, &key, RUNTIME_MANIFEST_JWS_TYPE).unwrap();
        assert!(matches!(
            verify_runtime_manifest(&unchecked, &keys, 0, beyond.issued_at),
            Err(ProtocolError::Invalid("runtime manifest shape"))
        ));
    }

    #[test]
    fn manifest_shape_and_pack_slots_are_bounded() {
        assert!(rejected(&changed(|value| value.version = 2)));
        assert!(rejected(&changed(|value| value.sequence = 0)));
        assert!(rejected(&changed(
            |value| value.sequence = MAX_SAFE_INTEGER + 1
        )));
        assert!(rejected(&changed(|value| value.issued_at = -1)));
        assert!(rejected(&changed(
            |value| value.expires_at = value.issued_at
        )));
        assert!(rejected(&changed(|value| value.packs.clear())));
        assert!(rejected(&changed(|value| {
            value.packs = vec![pack(ModelBackend::Cpu); MAX_RUNTIME_PACKS + 1];
            for (index, pack) in value.packs.iter_mut().enumerate() {
                pack.build = format!("b{index}");
            }
        })));
        let duplicate = changed(|value| value.packs[1].backend = ModelBackend::Cpu);
        assert!(matches!(
            validate_runtime_manifest(&duplicate),
            Err(ProtocolError::Invalid(
                "runtime pack listed twice for one build"
            ))
        ));
        let next_build = changed(|value| {
            value.packs[1] = pack(ModelBackend::Cpu);
            value.packs[1].build = "b10900".into();
        });
        validate_runtime_manifest(&next_build).unwrap();
        assert_eq!(
            next_build
                .find(
                    ModelRuntime::Llamacpp,
                    ReleaseTarget::LinuxX86_64,
                    ModelBackend::Cpu
                )
                .unwrap()
                .build,
            "b10809"
        );
    }

    #[test]
    fn runtime_target_and_backend_must_belong_together() {
        let mlx = |target, backend| {
            changed(|value| {
                value.packs.truncate(1);
                value.packs[0].runtime = ModelRuntime::Mlx;
                value.packs[0].target = target;
                value.packs[0].backend = backend;
            })
        };
        validate_runtime_manifest(&mlx(ReleaseTarget::MacosAarch64, ModelBackend::Metal)).unwrap();
        assert!(rejected(&mlx(
            ReleaseTarget::MacosX86_64,
            ModelBackend::Metal
        )));
        assert!(rejected(&mlx(
            ReleaseTarget::MacosAarch64,
            ModelBackend::Cpu
        )));
        assert!(rejected(&mlx(
            ReleaseTarget::LinuxAarch64,
            ModelBackend::Cpu
        )));
        let llamacpp = |target, backend| {
            changed(|value| {
                value.packs.truncate(1);
                value.packs[0].target = target;
                value.packs[0].backend = backend;
            })
        };
        for (target, backend) in [
            (ReleaseTarget::MacosAarch64, ModelBackend::Metal),
            (ReleaseTarget::MacosX86_64, ModelBackend::Cpu),
            (ReleaseTarget::LinuxAarch64, ModelBackend::Cpu),
            (ReleaseTarget::LinuxX86_64, ModelBackend::Cuda),
        ] {
            validate_runtime_manifest(&llamacpp(target, backend)).unwrap();
        }
        for (target, backend) in [
            (ReleaseTarget::MacosAarch64, ModelBackend::Vulkan),
            (ReleaseTarget::MacosX86_64, ModelBackend::Cuda),
            (ReleaseTarget::LinuxX86_64, ModelBackend::Metal),
        ] {
            assert!(rejected(&llamacpp(target, backend)));
        }
    }

    #[test]
    fn pack_identity_url_digest_and_size_are_canonical() {
        for build in ["", "../b1", "b1/x", ".b1", "b 1", &"b".repeat(65)] {
            assert!(
                rejected(&changed(|value| value.packs[0].build = build.into())),
                "{build:?}"
            );
        }
        validate_runtime_manifest(&changed(|value| {
            value.packs[0].build = "g1a2b3c4d5e6f".into()
        }))
        .unwrap();
        for url in [
            "http://cdn.example/pack.tar.gz",
            "https://cdn.example/pack.tar.gz?token=secret",
            "https://cdn.example/pack.tar.gz#part",
            "https://user:secret@cdn.example/pack.tar.gz",
            "https://cdn.example/a/../pack.tar.gz",
            "https://CDN.example/pack.tar.gz",
        ] {
            assert!(
                rejected(&changed(|value| value.packs[0].url = url.into())),
                "{url}"
            );
        }
        for digest in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
            assert!(rejected(&changed(
                |value| value.packs[0].sha256 = digest.clone()
            )));
            assert!(rejected(&changed(
                |value| value.packs[0].files[0].sha256 = digest.clone()
            )));
        }
        assert!(rejected(&changed(|value| value.packs[0].size = 0)));
        assert!(rejected(&changed(
            |value| value.packs[0].size = MAX_RUNTIME_PACK_BYTES + 1
        )));
    }

    fn with_files(paths: &[&str]) -> RuntimeManifest {
        changed(|value| {
            value.packs[0].files = paths
                .iter()
                .map(|path| file(path, *path == "llama-server"))
                .collect();
        })
    }

    #[test]
    fn pack_files_are_sorted_safe_relative_paths() {
        validate_runtime_manifest(&with_files(&[
            "mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib",
            "llama-server",
        ]))
        .unwrap_err();
        validate_runtime_manifest(&with_files(&[
            "llama-server",
            "mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib",
        ]))
        .unwrap();
        for paths in [
            &["llama-server", "llama-server"][..],
            &["Llama-server", "llama-server"],
            &["libggml.so", "libggml.so/inner", "llama-server"],
            &["../escape", "llama-server"],
            &["/absolute", "llama-server"],
            &["a//b", "llama-server"],
            &["a\\b", "llama-server"],
            &["dir/./x", "llama-server"],
            &["llama-server", "pack.json"],
            &["PACK.JSON", "llama-server"],
            &["llama server", "llama-server"],
            &["caf\u{e9}", "llama-server"],
            &["con.so", "llama-server"],
            &["libx.so.", "llama-server"],
        ] {
            assert!(rejected(&with_files(paths)), "{paths:?}");
        }
    }

    #[test]
    fn pack_files_are_bounded_and_include_an_executable_entrypoint() {
        assert!(rejected(&changed(
            |value| value.packs[0].entrypoint = "missing".into()
        )));
        assert!(rejected(&changed(
            |value| value.packs[0].files[3].executable = false
        )));
        assert!(rejected(&changed(|value| value.packs[0].files[1].size = 0)));
        assert!(rejected(&changed(|value| value.packs[0].files.clear())));
        assert!(rejected(&changed(|value| {
            value.packs[0].files = (0..=MAX_RUNTIME_PACK_FILES)
                .map(|index| file(&format!("lib{index:03}.so"), false))
                .collect();
        })));
        assert!(rejected(&changed(|value| {
            value.packs[0].files[1].size = MAX_RUNTIME_PACK_UNPACKED_BYTES;
        })));
        assert!(rejected(&changed(
            |value| value.packs[0].files[1].size = u64::MAX
        )));
    }

    #[test]
    fn manifests_reject_unknown_fields() {
        let mut json = serde_json::to_value(manifest()).unwrap();
        json["packs"][0]["files"][0]["mode"] = 0o4755.into();
        assert!(serde_json::from_value::<RuntimeManifest>(json).is_err());
        let mut json = serde_json::to_value(manifest()).unwrap();
        json["packs"][0]["arguments"] = "--rpc".into();
        assert!(serde_json::from_value::<RuntimeManifest>(json).is_err());
        let json = serde_json::to_value(manifest()).unwrap();
        assert_eq!(json["packs"][0]["runtime"], "llamacpp");
        assert_eq!(json["packs"][1]["backend"], "vulkan");
        assert_eq!(json["packs"][0]["target"], "x86_64-unknown-linux-gnu");
    }

    const LINUX_X64_FILES: &str = "THIRD-PARTY-NOTICES.txt fallback/libgcc_s.so.1 fallback/libstdc++.so.6 libcrypto.so.3 \
        libggml-base.so.0 libggml.so.0 libgomp.so.1 libllama-common.so.0 libllama-server-impl.so \
        libllama.so.0 libmtmd.so.0 libssl.so.3 llama-server libggml-cpu-alderlake.so \
        libggml-cpu-cannonlake.so libggml-cpu-cascadelake.so libggml-cpu-cooperlake.so \
        libggml-cpu-haswell.so libggml-cpu-icelake.so libggml-cpu-ivybridge.so \
        libggml-cpu-piledriver.so libggml-cpu-sandybridge.so libggml-cpu-sapphirerapids.so \
        libggml-cpu-skylakex.so libggml-cpu-sse42.so libggml-cpu-x64.so libggml-cpu-zen4.so";

    fn listed(vulkan: bool) -> Vec<RuntimePackFile> {
        let mut paths = LINUX_X64_FILES.split_whitespace().collect::<Vec<_>>();
        if vulkan {
            paths.extend(["fallback/libvulkan.so.1", "libggml-vulkan.so"]);
        }
        paths.sort();
        paths
            .into_iter()
            .map(|path| RuntimePackFile {
                size: 54_782_384,
                sha256: "d".repeat(64),
                ..file(path, path == "llama-server")
            })
            .collect()
    }

    /// The Linux x64 manifest is the largest one CI signs: a CPU and a Vulkan pack.
    fn linux_x64_release_manifest() -> RuntimeManifest {
        changed(|value| {
            value.sequence = MAX_SAFE_INTEGER;
            value.packs[0].files = listed(false);
            value.packs[1].files = listed(true);
            for pack in &mut value.packs {
                pack.url = format!(
                    "https://downloads.flow-like.example/standalone/releases/{MAX_SAFE_INTEGER}/runtimes/llamacpp-b10809-x86_64-unknown-linux-gnu-{}.tar.gz",
                    wire(pack.backend)
                );
            }
        })
    }

    #[test]
    fn the_largest_release_manifest_fits_the_compact_jws_cap() {
        let key = SigningKey::generate();
        let value = linux_x64_release_manifest();
        let signed = sign_runtime_manifest(&value, &key).unwrap();
        assert!(
            signed.len() < MAX_COMPACT_JWS_BYTES * 3 / 4,
            "{}",
            signed.len()
        );
        assert_eq!(
            verify_runtime_manifest(&signed, &[key.public_key()], 0, 1_000).unwrap(),
            value
        );
        let crowded = changed(|value| {
            value.packs = (0..6)
                .map(|index| {
                    let mut pack = linux_x64_release_manifest().packs[1].clone();
                    pack.build = format!("b{index}");
                    pack
                })
                .collect();
        });
        validate_runtime_manifest(&crowded).unwrap();
        assert!(matches!(
            sign_runtime_manifest(&crowded, &key),
            Err(ProtocolError::Invalid("JWS size"))
        ));
    }
}
