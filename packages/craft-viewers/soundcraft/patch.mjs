import { readFileSync } from "node:fs";
import { join } from "node:path";
import { prepareLiveMcp } from "../live-mcp.mjs";

export function assertReviewedModules(source, expected, label) {
	const modules = [
		...source.matchAll(/^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*[;{]/gm),
	]
		.map((match) => match[1])
		.sort();
	if (JSON.stringify(modules) !== JSON.stringify([...expected].sort()))
		throw new Error(
			`${label} modules changed; review the browser codec boundary.`,
		);
}

export default function prepare({
	source,
	directory,
	original,
	write,
	replaceOnce,
}) {
	const audio = original("crates/audio-io/src/lib.rs");
	assertReviewedModules(
		audio,
		["aiff", "buffer", "caf", "flac", "mp4", "pcm", "peaks", "symph", "wav"],
		"SoundCraft audio",
	);
	// Keep software compressed codecs outside the selected WASM dependency graph.
	write(
		"crates/audio-io/Cargo.toml",
		replaceOnce(
			original("crates/audio-io/Cargo.toml"),
			"symphonia.workspace = true",
			`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]\nsymphonia.workspace = true`,
			"native-only Symphonia",
		),
	);
	write(
		"crates/audio-io/src/lib.rs",
		replaceOnce(
			audio,
			"mod symph;",
			'#[cfg(not(target_arch = "wasm32"))]\nmod symph;\n#[cfg(target_arch = "wasm32")]\n#[path = "browser_codec.rs"]\nmod symph;',
			"browser audio decoder boundary",
		),
	);
	write(
		"crates/audio-io/src/browser_codec.rs",
		readFileSync(join(directory, "browser-codec.rs"), "utf8"),
	);
	let engineIo = original("crates/engine/src/io.rs");
	engineIo += `
/// The active reader's PCM size, without decoding the complete file.
pub fn decoded_audio_byte_length(bytes: &[u8], ext: Option<&str>) -> Result<u64> {
    let info = soundcraft_audio_io::probe(bytes, ext).map_err(|err| EngineError::Io(err.to_string()))?;
    Ok(info.frames.saturating_mul(u64::from(info.channels)).saturating_mul(4))
}
`;
	engineIo += `
/// Unpack browser media using the same limits as the session importer.
pub fn read_host_bundle(bytes: &[u8]) -> Result<(String, std::collections::HashMap<String, Vec<u8>>)> {
    read_bundle(bytes, bundle::BUNDLE_LIMITS)
}
pub use bundle::open_session_bytes_with_audio;
`;
	write("crates/engine/src/io.rs", engineIo);
	let bundle = original("crates/engine/src/io/bundle.rs");
	bundle = replaceOnce(
		bundle,
		"const BUNDLE_LIMITS:",
		"pub(super) const BUNDLE_LIMITS:",
		"host bundle limits",
	);
	bundle = replaceOnce(
		bundle,
		"pub fn open_session_bytes(e: &mut Engine, name: &str, bytes: &[u8]) -> Result<Vec<String>> {",
		`pub fn open_session_bytes(e: &mut Engine, name: &str, bytes: &[u8]) -> Result<Vec<String>> {
    open_session_bytes_with_audio(e, name, bytes, std::collections::HashMap::new())
}

/// Open a bundle with audio already decoded by the host browser.
pub fn open_session_bytes_with_audio(e: &mut Engine, name: &str, bytes: &[u8], decoded: std::collections::HashMap<String, Vec<u8>>) -> Result<Vec<String>> {`,
		"browser-decoded session audio",
	);
	bundle = replaceOnce(
		bundle,
		'    let mut s = Session::from_json(&text).map_err(|err| EngineError::Io(format!("{name}: {err}")))?;\n    let mut missing = Vec::new();',
		'    files.extend(decoded);\n    let mut s = Session::from_json(&text).map_err(|err| EngineError::Io(format!("{name}: {err}")))?;\n    let mut missing = Vec::new();',
		"prepared session media",
	);
	write("crates/engine/src/io/bundle.rs", bundle);
	// Audio does not expose video import. Exclude the copied H.264/ProRes code,
	// including synthetic encoders, instead of leaving unreachable codecs linked.
	let video = original("crates/video/src/lib.rs");
	assertReviewedModules(
		video,
		["convert", "h264", "movie", "prores", "synth", "tests"],
		"SoundCraft video",
	);
	for (const module of ["h264", "prores", "synth"]) {
		video = replaceOnce(
			video,
			`pub mod ${module};`,
			`#[cfg(not(target_arch = "wasm32"))]\npub mod ${module};`,
			`native-only ${module}`,
		);
	}
	video = replaceOnce(
		video,
		"mod movie;",
		'#[cfg(not(target_arch = "wasm32"))]\nmod movie;\n#[cfg(target_arch = "wasm32")]\n#[path = "browser_movie.rs"]\nmod movie;',
		"browser video boundary",
	);
	write("crates/video/src/lib.rs", video);
	const movie = original("crates/video/src/movie.rs");
	const marker = "\nenum Codec {";
	if (
		movie.indexOf(marker) < 0 ||
		movie.indexOf(marker) !== movie.lastIndexOf(marker)
	)
		throw new Error(
			"SoundCraft movie types changed; review the browser codec boundary.",
		);
	let movieTypes = movie.slice(0, movie.indexOf(marker));
	for (const [search, replacement] of [
		[
			"use crate::convert::{Matrix, Planes, planar_to_rgba, scale_rgba};",
			"use crate::convert::scale_rgba;",
		],
		[
			"use crate::isobmff::{self, ByteSource, CodecConfig, Mp4File, TrackKind};",
			"use crate::isobmff::ByteSource;",
		],
		["use std::collections::VecDeque;\n", ""],
		[
			"/// Largest picture we decode (bounds every allocation derived from the file).\npub(crate) const MAX_PIXELS: u64 = 8192 * 8192;\n",
			"",
		],
	])
		movieTypes = replaceOnce(
			movieTypes,
			search,
			replacement,
			"browser movie types",
		);
	write(
		"crates/video/src/browser_movie.rs",
		movieTypes + readFileSync(join(directory, "browser-movie.rs"), "utf8"),
	);
	write(
		"apps/soundcraft-web/src/audio.js",
		readFileSync(join(directory, "audio.js"), "utf8"),
	);
	write(
		"apps/soundcraft-web/src/web.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	let services = original("crates/ui-egui/src/services.rs");
	services = replaceOnce(
		services,
		"pub struct Services {",
		"pub struct Services {\n    pub host_command: Option<Box<dyn Fn(&str) -> Option<Result<serde_json::Value, String>>>>,",
		"host services",
	);
	write("crates/ui-egui/src/services.rs", services);
	let ui = original("crates/ui-egui/src/app/commands.rs");
	ui = replaceOnce(
		ui,
		"pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {",
		"pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {\n        if let Some(result) = self.services.host_command.as_ref().and_then(|hook| hook(id)) { return result; }",
		"command interception",
	);
	write("crates/ui-egui/src/app/commands.rs", ui);
	write(
		"crates/ui-egui/src/menus.rs",
		original("crates/ui-egui/src/menus.rs")
			.replaceAll('"About SoundCraft"', '"About Flows Audio"')
			.replaceAll('"SoundCraft"', '"Flows Audio"'),
	);
	write(
		"crates/ui-egui/src/panels/about.rs",
		original("crates/ui-egui/src/panels/about.rs")
			.replaceAll('"About SoundCraft"', '"About Flows Audio"')
			.replaceAll('"SoundCraft"', '"Flows Audio"')
			.replace(
				'"An open-source digital audio workstation from the ArtCraft team."',
				'"Audio editing in Flow-Like, powered by SoundCraft."',
			),
	);
	prepareLiveMcp({ id: "soundcraft", source, write, original, replaceOnce });
}
