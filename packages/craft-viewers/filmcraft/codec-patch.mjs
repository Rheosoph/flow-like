import { readFileSync } from "node:fs";
import { join } from "node:path";
import prepareBrowserAudio from "../pcm-codec-patch.mjs";

export default function prepareBrowserCodecs({
	root,
	directory,
	original,
	write,
	replaceOnce,
}) {
	const local = (name) => readFileSync(join(directory, name), "utf8");
	const between = (source, start, end, replacement, label) => {
		const first = source.indexOf(start);
		const last = source.indexOf(end, first + start.length);
		if (
			first < 0 ||
			last < 0 ||
			source.indexOf(start, first + start.length) >= 0
		)
			throw new Error(`Film browser codec patch needs review: ${label}`);
		return source.slice(0, first) + replacement + source.slice(last);
	};
	write("apps/filmcraft-web/src/browser-codecs.js", local("browser-codecs.js"));
	write("apps/filmcraft-web/src/webcodecs.rs", local("browser-media.rs"));
	write("apps/filmcraft-web/src/browser_export.rs", local("browser-export.rs"));
	let webCargo = original("apps/filmcraft-web/Cargo.toml");
	webCargo = replaceOnce(
		webCargo,
		"filmcraft-engine = { workspace = true }",
		"filmcraft-engine = { workspace = true }\nfilmcraft-project = { workspace = true }\nfilmcraft-render = { workspace = true }\nfilmcraft-matroska = { workspace = true }",
		"browser export compositor dependencies",
	);
	write("apps/filmcraft-web/Cargo.toml", webCargo);
	// The replacement registry has no decoder implementation dependencies.
	write("crates/codecs/src/lib.rs", local("browser-registry.rs"));
	let codecsCargo = original("crates/codecs/Cargo.toml");
	codecsCargo = between(
		codecsCargo,
		"[dependencies]\n",
		"[dev-dependencies]\n",
		"[dependencies]\nfilmcraft-media = { workspace = true }\n\n",
		"codec dependency isolation",
	);
	write("crates/codecs/Cargo.toml", codecsCargo);
	let perf = original("crates/engine/src/perf.rs");
	perf = between(
		perf,
		"pub fn decode_json() -> Value {",
		"/// Export counters:",
		`pub fn decode_json() -> Value {
    json!({"backend": "browser", "softwareFallback": false})
}

`,
		"browser decoder diagnostics",
	);
	write("crates/engine/src/perf.rs", perf);
	let exportCargo = original("crates/export/Cargo.toml");
	exportCargo = replaceOnce(
		exportCargo,
		"craft-video = { workspace = true }\n",
		"",
		"software video encoder dependency",
	);
	let workspace = original("Cargo.toml");
	const audio = workspace.match(/^craft-audio = .*$/m)?.[0];
	if (!audio) throw new Error("Film shared audio dependency changed.");
	workspace = replaceOnce(
		workspace,
		audio,
		audio.replace(
			/features = \[[^\]]*\]/,
			'features = ["pcm-encode", "flac-encode"]',
		),
		"browser audio features",
	);
	const pinnedSourceOverrides = prepareBrowserAudio({
		id: "filmcraft",
		root,
		original,
		write,
		replaceOnce,
		workspace,
	});
	write("crates/export/Cargo.toml", exportCargo);
	let encoders = original("crates/export/src/encoders.rs");
	encoders = between(
		encoders,
		"/// AAC-LC (our encoder),",
		"/// FLAC (our encoder)",
		"",
		"remove AAC implementation",
	);
	const software = encoders.indexOf("pub(crate) fn aac_factory(");
	if (software < 0) throw new Error("Film software encoder boundary changed.");
	write("crates/export/src/encoders.rs", encoders.slice(0, software));
	write(
		"crates/export/src/lib.rs",
		replaceOnce(
			original("crates/export/src/lib.rs"),
			"pub use encoders::{apv_profile, dnx_profile, prores_profile};",
			"",
			"software encoder profile exports",
		),
	);
	let registry = original("crates/export/src/registry.rs");
	registry = replaceOnce(
		registry,
		"use super::encoders::{aac_factory, apv_factory, dnx_factory, h264_factory, mjpeg_factory, prores_factory};",
		"use super::encoders::mjpeg_factory;",
		"software encoder imports",
	);
	registry = registry.replaceAll("craft_video::h264::PassStats", "()");
	registry = replaceOnce(
		registry,
		"vec![h264_factory, prores_factory, dnx_factory, apv_factory, mjpeg_factory]",
		"vec![mjpeg_factory]",
		"native encoder registry",
	);
	registry = replaceOnce(
		registry,
		"vec![aac_factory]",
		"Vec::new()",
		"native audio encoder registry",
	);
	write("crates/export/src/registry.rs", registry);
	let format = original("crates/export/src/format.rs").replaceAll(
		"craft_video::h264::PassStats",
		"()",
	);
	format = replaceOnce(
		format,
		"!matches!(self, Format::Hevc | Format::Av1)",
		"!matches!(self, Format::H264 | Format::Hevc | Format::Av1 | Format::Aac | Format::ProRes | Format::DnxHr | Format::Apv | Format::MxfOp1a | Format::MxfOpAtom | Format::Mp3)",
		"unsupported export formats",
	);
	write("crates/export/src/format.rs", format);
	let settings = original("crates/export/src/settings.rs");
	settings = replaceOnce(
		settings,
		"crate::prores_profile(&self.prores_profile).chroma() == craft_video::prores::ChromaFormat::Yuv444",
		"false",
		"ProRes alpha availability",
	);
	settings = replaceOnce(
		settings,
		"crate::prores_profile(&self.prores_profile).nominal_mbps_1080p30()",
		"220.0",
		"unused ProRes estimate",
	);
	settings = between(
		settings,
		"let bpp = match crate::apv_profile(&self.apv_profile) {",
		"                px * fps * bpp",
		"let bpp = 3.0;\n",
		"unused APV estimate",
	);
	settings = between(
		settings,
		"                    use craft_video::prores::Profile;",
		"                Format::DnxHr =>",
		'                    v += ", unavailable in this browser build";\n                }\n',
		"ProRes export summary",
	);
	settings = replaceOnce(
		settings,
		'Format::Apv => v += &format!(", {}", crate::apv_profile(&self.apv_profile).name()),',
		'Format::Apv => v += ", unavailable in this browser build",',
		"APV export summary",
	);
	settings = between(
		settings,
		"        let rate = craft_audio::mp3::nearest_sample_rate",
		"        (rate, channels, kbps)",
		"        let rate = self.audio.sample_rate.unwrap_or(seq_sr);\n        let channels = self.audio.channels.clamp(1, 2);\n        let kbps = self.audio.bitrate_kbps;\n",
		"unused MP3 settings",
	);
	const unavailableAudio = `pub(crate) fn export(
    _project: &std::sync::Arc<filmcraft_project::Project>,
    _seq: filmcraft_project::ItemId,
    _settings: &crate::ExportSettings,
    _range: filmcraft_time::TimeRange,
    _sources: &dyn filmcraft_render::SourceProvider,
    _progress: &crate::Progress,
) -> crate::Result<(u64, u64)> {
    Err(crate::ExportError::Unsupported("Use the browser export controls.".into()))
}
`;
	write("crates/export/src/mp3_out.rs", unavailableAudio);
	write(
		"crates/export/src/aac_out.rs",
		`${unavailableAudio}pub(crate) fn is_adts_path(path: &str) -> bool { std::path::Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("aac")) }\n`,
	);
	write("crates/export/src/settings.rs", settings);
	let mxf = original("crates/export/src/mxf_out.rs");
	mxf = replaceOnce(
		mxf,
		"Format, H264Profile, Out",
		"Format, Out",
		"remove unused encoder profile import",
	);
	mxf = between(
		mxf,
		"    let coding = match settings.video_format() {",
		"    let mut d = PictureDesc::new(coding, w, h);",
		"    let coding = PictureCoding::Avc { profile_idc: 100, intra: false };\n",
		"remove unused MXF codec metadata dependencies",
	);
	write("crates/export/src/mxf_out.rs", mxf);
	let manager = original("crates/engine/src/project_manager.rs");
	manager = replaceOnce(
		manager,
		"filmcraft_export::prores_profile(pr.prores).nominal_mbps_1080p30()",
		"220.0",
		"unused proxy estimate",
	);
	write("crates/engine/src/project_manager.rs", manager);
	return pinnedSourceOverrides;
}
