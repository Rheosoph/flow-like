import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const directory = dirname(fileURLToPath(import.meta.url));

const revisions = {
	soundcraft: "a3fca1707941bce4768586661ccbb94c44ee46eb",
	filmcraft: "33be35625e252cc4f06aa375b3b428521faea37d",
};
const repository = "https://github.com/storytold/craft-libs";
const packagePath = "host-dependencies/craft-audio";

export default function prepareBrowserAudio({
	id = "soundcraft",
	root,
	original,
	write,
	replaceOnce,
	workspace = original("Cargo.toml"),
}) {
	const revision = revisions[id];
	if (!revision)
		throw new Error("No reviewed PCM audio dependency for this editor.");
	const checkout = join(
		root,
		"target/craft-viewers",
		id,
		"dependencies",
		revision,
	);
	mkdirSync(checkout, { recursive: true });
	const git = (args, encoding = "utf8") =>
		execFileSync("git", args, {
			cwd: checkout,
			encoding,
			maxBuffer: 32 * 1024 * 1024,
		});
	if (!existsSync(join(checkout, ".git"))) git(["init", "--quiet"]);
	try {
		git(["cat-file", "-e", `${revision}^{commit}`]);
	} catch {
		git(["fetch", "--depth", "1", repository, revision]);
	}
	const read = (path) => git(["show", `${revision}:${path}`]);
	// Read immutable Git objects so a local checkout cannot alter the browser codec boundary.
	const files = git([
		"ls-tree",
		"-r",
		"--name-only",
		revision,
		"crates/audio/src",
	])
		.trim()
		.split("\n");
	for (const file of files)
		write(
			`${packagePath}/${file.slice("crates/audio/".length)}`,
			git(["show", `${revision}:${file}`], null),
		);
	for (const license of ["LICENSE-MIT", "LICENSE-APACHE"])
		write(`${packagePath}/${license}`, read(license));
	let cargo = read("crates/audio/Cargo.toml");
	cargo = cargo.slice(0, cargo.indexOf("[dev-dependencies]"));
	for (const [name, value] of Object.entries({
		version: '"0.1.0"',
		edition: '"2024"',
		license: '"MIT OR Apache-2.0"',
		"rust-version": '"1.95"',
		repository: JSON.stringify(repository),
	})) {
		cargo = replaceOnce(
			cargo,
			`\n${name}.workspace = true\n`,
			`\n${name} = ${value}\n`,
			`shared audio ${name}`,
		);
	}
	cargo = replaceOnce(
		cargo,
		"[features]\n",
		'[features]\npcm-read = ["dep:craft-xml", "dep:log"]\n',
		"PCM reader feature",
	);
	for (const name of ["craft-media", "craft-xml"])
		cargo = cargo.replace(
			`${name} = { workspace = true`,
			`${name} = { git = "${repository}", rev = "${revision}"`,
		);
	for (const [name, version] of [
		["thiserror", "2"],
		["serde", "1"],
		["rayon", "1"],
	])
		cargo = cargo.replace(
			`${name} = { workspace = true`,
			`${name} = { version = "${version}"`,
		);
	cargo = replaceOnce(
		cargo,
		`craft-media = { git = "${repository}", rev = "${revision}" }`,
		`craft-media = { git = "${repository}", rev = "${revision}", default-features = false }`,
		"shared media feature defaults",
	);
	cargo = replaceOnce(
		cargo,
		'serde = { version = "1", optional = true }',
		'serde = { version = "1", features = ["derive"], optional = true }',
		"shared serde derives",
	);
	write(`${packagePath}/Cargo.toml`, cargo);
	let lib = read("crates/audio/src/lib.rs");
	lib = lib.replaceAll('feature = "decode-native"', 'feature = "pcm-read"');
	lib = replaceOnce(
		lib,
		'    feature = "aac-decode",\n    feature = "alac-decode",\n    feature = "opus-decode",\n    feature = "ac3-decode",\n    feature = "mpa-decode",\n    feature = "flac-decode"\n))]\nmod pcm;',
		'    feature = "pcm-read"\n))]\nmod pcm;',
		"PCM helpers",
	);
	lib = replaceOnce(
		lib,
		'#[cfg(feature = "pcm-read")]\npub use read::SharedSource;\n',
		"",
		"unused streaming reader export",
	);
	lib = replaceOnce(
		lib,
		"pub use read::{\n    DECODE_EXTENSIONS, DEFAULT_MAX_SAMPLES, DecodeOptions, Decoder, TrackInfo, decode, decode_with, detect_container, probe, probe_with, tracks,\n    tracks_of,\n};",
		"pub use read::{DECODE_EXTENSIONS, decode, probe};",
		"browser reader API",
	);
	write(`${packagePath}/src/lib.rs`, lib);
	write(
		`${packagePath}/src/pcm.rs`,
		read("crates/audio/src/pcm.rs").replaceAll(
			'feature = "decode-native"',
			'feature = "pcm-read"',
		),
	);
	write(
		`${packagePath}/src/read/mod.rs`,
		readFileSync(join(directory, "browser-pcm.rs"), "utf8"),
	);
	const audioDependency = workspace.match(/^craft-audio = .*$/m)?.[0];
	if (!audioDependency)
		throw new Error("Missing pinned shared audio dependency.");
	workspace = replaceOnce(
		workspace,
		audioDependency,
		audioDependency.replace(
			/features = \[[^\]]*\]/,
			'features = ["pcm-read", "lossless-encode", "peaks", "serde"]',
		),
		"browser audio features",
	);
	const aafPath = "host-dependencies/craft-aaf";
	for (const file of git([
		"ls-tree",
		"-r",
		"--name-only",
		revision,
		"crates/aaf/src",
	])
		.trim()
		.split("\n"))
		write(
			`${aafPath}/${file.slice("crates/aaf/".length)}`,
			git(["show", `${revision}:${file}`], null),
		);
	for (const license of ["LICENSE-MIT", "LICENSE-APACHE"])
		write(`${aafPath}/${license}`, read(license));
	let aaf = read("crates/aaf/Cargo.toml");
	for (const [name, value] of Object.entries({
		version: '"0.1.0"',
		edition: '"2024"',
		license: '"MIT OR Apache-2.0"',
		"rust-version": '"1.95"',
		repository: JSON.stringify(repository),
	}))
		aaf = replaceOnce(
			aaf,
			`\n${name}.workspace = true\n`,
			`\n${name} = ${value}\n`,
			`shared AAF ${name}`,
		);
	for (const name of ["craft-cfb", "craft-timeline", "craft-audio"])
		aaf = aaf.replace(
			`${name} = { workspace = true`,
			`${name} = { git = "${repository}", rev = "${revision}"`,
		);
	aaf = replaceOnce(
		aaf,
		'features = ["decode-native", "pcm-encode"]',
		'features = ["pcm-read", "pcm-encode"]',
		"AAF PCM-only input",
	);
	aaf = replaceOnce(
		aaf,
		"thiserror = { workspace = true }",
		'thiserror = { version = "2" }',
		"AAF error dependency",
	);
	write(`${aafPath}/Cargo.toml`, aaf);
	workspace += `\n[patch."${repository}"]\ncraft-audio = { path = "${packagePath}" }\ncraft-aaf = { path = "${aafPath}" }\n`;
	write("Cargo.toml", workspace);
	return ["craft-audio", "craft-aaf"].map((name) => ({
		name,
		version: "0.1.0",
		source: `git+${repository}?rev=${revision}#${revision}`,
		path: `host-dependencies/${name}`,
	}));
}
