import assert from "node:assert/strict";
import {
	mkdirSync,
	mkdtempSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
	assertPinnedLockPackages,
	assertPlatformMediaDependencies,
} from "./codec-policy.mjs";

test("platform media builds retain containers, rendering and lossless writers", () => {
	assert.doesNotThrow(() =>
		assertPlatformMediaDependencies(
			"filmcraft",
			[
				"filmcraft-web v0.7.0 (/source/apps/filmcraft-web)",
				"filmcraft-isobmff v0.7.0 (/source/crates/isobmff)",
				"filmcraft-mpegts v0.7.0 (/source/crates/mpegts)",
				"filmcraft-frame v0.7.0 (/source/crates/frame)",
				"filmcraft-flac v0.7.0 (/source/crates/flac)",
				"wasm-bindgen v0.2.129 (*)",
			].join("\n"),
			"filmcraft-web",
		),
	);
});

test("rejects direct and transitive software codec dependencies", () => {
	for (const codec of [
		"symphonia",
		"symphonia-codec-aac",
		"symphonia-bundle-mp3",
		"deckcraft-h264",
		"deckcraft-av1",
		"deckcraft-opus",
		"filmcraft-hevc",
		"filmcraft-h264enc",
		"filmcraft-dnx",
		"filmcraft-ac3",
		"filmcraft-prores",
		"filmcraft-apv",
		"filmcraft-vp9",
		"ffmpeg-next",
		"openh264-sys",
		"rav1e",
		"future-mp3",
		"future-mp2",
		"future-vorbis",
		"alac",
		"minimp3-sys",
		"puremp3",
		"rmp3",
		"lewton",
		"audiopus",
		"mp3lame-encoder",
	]) {
		assert.throws(
			() =>
				assertPlatformMediaDependencies(
					"deckcraft",
					`deckcraft-web v0.6.0\n${codec} v1.0.0`,
					"deckcraft-web",
				),
			/browser\/OS media codecs/,
			codec,
		);
	}
});

test("an empty, malformed or unrelated graph cannot pass the codec check", () => {
	for (const tree of ["", "not a Cargo graph", "some-other-app v1.0.0"])
		assert.throws(() =>
			assertPlatformMediaDependencies("soundcraft", tree, "soundcraft-web"),
		);
});

test("shared media features retain lossless writers and reject embedded codecs", () => {
	const root = "soundcraft-web v0.20.1 features=[]";
	const safe =
		"craft-audio v0.1.0 (https://example.com/pinned) features=[flac-encode,lossless-encode,pcm-encode,peaks,serde]";
	assert.doesNotThrow(() =>
		assertPlatformMediaDependencies(
			"soundcraft",
			`${root}\n${safe}\n${safe} (*)`,
			"soundcraft-web",
		),
	);
	for (const name of ["craft-audio", "craft-video"]) {
		for (const feature of [
			"decode",
			"decode-native",
			"aac-decode",
			"h264",
			"future-codec",
		]) {
			assert.throws(
				() =>
					assertPlatformMediaDependencies(
						"soundcraft",
						`${root}\n${name} v0.1.0 features=[${feature}]`,
						"soundcraft-web",
					),
				/shared media features/,
			);
		}
		assert.throws(
			() =>
				assertPlatformMediaDependencies(
					"soundcraft",
					`${root}\n${name} v0.1.0`,
					"soundcraft-web",
				),
			/activated features/,
		);
	}
});

test("lock reconciliation can remove codecs without upgrading dependencies", () => {
	const original = `version = 4
[[package]]
name = "editor-web"
version = "1.0.0"
dependencies = ["software-codec", "wasm-bindgen"]
[[package]]
name = "software-codec"
version = "1.0.0"
[[package]]
name = "wasm-bindgen"
version = "0.2.129"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123"
`;
	const reconciled = original
		.replace('"software-codec", ', "")
		.replace('[[package]]\nname = "software-codec"\nversion = "1.0.0"\n', "");
	assert.doesNotThrow(() => assertPinnedLockPackages(original, reconciled));
	for (const changed of [
		reconciled.replace('"0.2.129"', '"0.2.130"'),
		reconciled.replace('checksum = "abc123"', 'checksum = "changed"'),
		reconciled.replace("registry+https:", "git+https:"),
		`${reconciled}\n[[package]]\nname = "new-codec"\nversion = "1.0.0"`,
		"",
	])
		assert.throws(() => assertPinnedLockPackages(original, changed));
});

const mediaRevision = "a3fca1707941bce4768586661ccbb94c44ee46eb";
const mediaSource = (revision = mediaRevision) =>
	`git+https://github.com/storytold/craft-libs?rev=${revision}#${revision}`;
const locked = (name, source = null, version = "0.1.0", checksum = null) =>
	`[[package]]\nname = ${JSON.stringify(name)}\nversion = ${JSON.stringify(version)}\n${source ? `source = ${JSON.stringify(source)}\n` : ""}${checksum ? `checksum = ${JSON.stringify(checksum)}\n` : ""}`;

function overrideFixture(t, name = "craft-audio", revision = mediaRevision) {
	const directory = mkdtempSync(join(tmpdir(), "craft-source-override-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	const source = join(directory, "source");
	const path = `host-dependencies/${name}`;
	const local = join(source, path);
	mkdirSync(local, { recursive: true });
	const manifest = join(local, "Cargo.toml");
	writeFileSync(
		manifest,
		`[package]\nname = "${name}"\nversion = "0.1.0"\n\n[dependencies]\n`,
	);
	const override = {
		name,
		version: "0.1.0",
		source: mediaSource(revision),
		path,
	};
	const unchanged = locked(
		"wasm-bindgen",
		"registry+https://github.com/rust-lang/crates.io-index",
		"0.2.129",
		"pinned-checksum",
	);
	const original = locked(name, override.source) + unchanged;
	const reconciled = locked(name) + unchanged;
	return { directory, source, local, manifest, override, original, reconciled };
}

const inventoryRevision = "ffc228473d7aa20ca0790ddccc894a20682489ae";
const inventorySource = `git+https://github.com/dtolnay/inventory?rev=${inventoryRevision}#${inventoryRevision}`;
const inventoryRegistry =
	"registry+https://github.com/rust-lang/crates.io-index";
const inventoryChecksum =
	"6928282826c822ad91bf1c9a1cb90a30ba1c26770749929b4656cd6be829cd7c";

function inventoryFixture(t) {
	const fixture = overrideFixture(t, "inventory");
	fixture.override.version = "0.3.25";
	fixture.override.source = inventorySource;
	writeFileSync(
		fixture.manifest,
		'[package]\nname = "inventory"\nversion = "0.3.25"\n',
	);
	fixture.original = locked(
		"inventory",
		inventoryRegistry,
		"0.3.25",
		inventoryChecksum,
	);
	fixture.reconciled = locked("inventory", null, "0.3.25");
	return fixture;
}

test("the reviewed inventory patch retains Git provenance from its exact registry release", (t) => {
	const f = inventoryFixture(t);
	assert.deepEqual(
		assertPinnedLockPackages(f.original, f.reconciled, {
			source: f.source,
			pinnedSourceOverrides: [f.override],
		}),
		[{ ...f.override, path: realpathSync(f.local) }],
	);
	assert.throws(() => assertPinnedLockPackages(f.original, f.reconciled));
});

test("inventory requires the reviewed registry checksum, version and Git revision", (t) => {
	const f = inventoryFixture(t);
	for (const original of [
		f.original.replace(inventoryChecksum, "0".repeat(64)),
		f.original.replace(`checksum = "${inventoryChecksum}"\n`, ""),
		f.original.replace("0.3.25", "0.3.24"),
		f.original.replace(inventoryRegistry, inventorySource),
		f.original.replace(inventoryRegistry, "registry+https://example.com/index"),
	])
		assert.throws(
			() =>
				assertPinnedLockPackages(original, f.reconciled, {
					source: f.source,
					pinnedSourceOverrides: [f.override],
				}),
			/Unreviewed pinned source override/,
		);
	for (const change of [
		{ name: "another-package" },
		{ version: "0.3.26" },
		{ source: inventoryRegistry },
		{ source: inventorySource.replace("dtolnay", "other") },
		{
			source: inventorySource.replace(`rev=${inventoryRevision}`, "rev=master"),
		},
		{
			source: inventorySource.replace(
				`#${inventoryRevision}`,
				`#${"1".repeat(40)}`,
			),
		},
		{ source: inventorySource.replaceAll(inventoryRevision, "2".repeat(40)) },
	])
		assert.throws(
			() =>
				assertPinnedLockPackages(f.original, f.reconciled, {
					source: f.source,
					pinnedSourceOverrides: [{ ...f.override, ...change }],
				}),
			/Unreviewed pinned source override/,
		);
});

test("inventory reconciliation requires exactly one local identity and allows no unrelated packages", (t) => {
	const f = inventoryFixture(t);
	for (const reconciled of [
		f.original,
		f.reconciled.replace("0.3.25", "0.3.26"),
		locked("inventory", inventorySource, "0.3.25"),
		locked("inventory", null, "0.3.25", inventoryChecksum),
		f.reconciled + f.reconciled,
		f.reconciled + locked("unreviewed-local", null, "0.3.25"),
	])
		assert.throws(() =>
			assertPinnedLockPackages(f.original, reconciled, {
				source: f.source,
				pinnedSourceOverrides: [f.override],
			}),
		);
	assert.throws(() =>
		assertPinnedLockPackages(f.original, f.reconciled, {
			source: f.source,
			pinnedSourceOverrides: [f.override, f.override],
		}),
	);
});

test("inventory paths and manifests stay inside the prepared source", (t) => {
	const f = inventoryFixture(t);
	const outside = join(f.directory, "outside");
	mkdirSync(outside);
	writeFileSync(
		join(outside, "Cargo.toml"),
		'[package]\nname = "inventory"\nversion = "0.3.25"\n',
	);
	symlinkSync(outside, join(f.source, "escape"));
	for (const path of ["../outside", outside, "escape", "."]) {
		assert.throws(
			() =>
				assertPinnedLockPackages(f.original, f.reconciled, {
					source: f.source,
					pinnedSourceOverrides: [{ ...f.override, path }],
				}),
			/inside the prepared source/,
		);
	}
	writeFileSync(
		f.manifest,
		'[package]\nname = "inventory"\nversion = "0.3.24"\n',
	);
	assert.throws(
		() =>
			assertPinnedLockPackages(f.original, f.reconciled, {
				source: f.source,
				pinnedSourceOverrides: [f.override],
			}),
		/manifest must declare its pinned version/,
	);
	rmSync(f.manifest);
	symlinkSync(join(outside, "Cargo.toml"), f.manifest);
	assert.throws(
		() =>
			assertPinnedLockPackages(f.original, f.reconciled, {
				source: f.source,
				pinnedSourceOverrides: [f.override],
			}),
		/manifest must stay inside/,
	);
});

test("reviewed local media patches retain their exact original Git identities", (t) => {
	for (const revision of [
		mediaRevision,
		"33be35625e252cc4f06aa375b3b428521faea37d",
	]) {
		for (const name of ["craft-audio", "craft-aaf"]) {
			const fixture = overrideFixture(t, name, revision);
			const local = assertPinnedLockPackages(
				fixture.original,
				fixture.reconciled,
				{
					source: fixture.source,
					pinnedSourceOverrides: [fixture.override],
				},
			);
			assert.equal(local.length, 1);
			assert.equal(local[0].name, name);
			assert.equal(local[0].version, "0.1.0");
			assert.equal(local[0].source, mediaSource(revision));
			assert.throws(() =>
				assertPinnedLockPackages(fixture.original, fixture.reconciled),
			);
		}
	}
});

test("source overrides cannot change another identity or lose their own checksum rules", (t) => {
	const f = overrideFixture(t);
	const options = { source: f.source, pinnedSourceOverrides: [f.override] };
	for (const reconciled of [
		f.reconciled.replace('"0.2.129"', '"0.2.130"'),
		f.reconciled.replace('"pinned-checksum"', '"changed"'),
		f.reconciled.replace('version = "0.1.0"', 'version = "0.1.1"'),
		f.reconciled.replace(
			'version = "0.1.0"',
			'version = "0.1.0"\nchecksum = "changed"',
		),
		f.reconciled + locked("unreviewed-local"),
		f.reconciled + locked("craft-audio"),
		f.original,
	])
		assert.throws(() =>
			assertPinnedLockPackages(f.original, reconciled, options),
		);
	assert.throws(() =>
		assertPinnedLockPackages(f.original, f.reconciled, {
			...options,
			pinnedSourceOverrides: [f.override, f.override],
		}),
	);
});

test("source overrides require the reviewed full SHA and an exact original lock entry", (t) => {
	const f = overrideFixture(t);
	for (const change of [
		{ name: "unreviewed-local" },
		{ version: "0.1.1" },
		{ source: mediaSource("1".repeat(40)) },
		{ source: f.override.source.replace(`rev=${mediaRevision}`, "rev=main") },
		{
			source: f.override.source.replace(
				`#${mediaRevision}`,
				`#${"2".repeat(40)}`,
			),
		},
		{ source: f.override.source.replace("storytold", "other") },
	])
		assert.throws(() =>
			assertPinnedLockPackages(f.original, f.reconciled, {
				source: f.source,
				pinnedSourceOverrides: [{ ...f.override, ...change }],
			}),
		);
	assert.throws(() =>
		assertPinnedLockPackages(
			f.original.replace(mediaRevision, "3".repeat(40)),
			f.reconciled,
			{
				source: f.source,
				pinnedSourceOverrides: [f.override],
			},
		),
	);
});

test("source overrides and their manifests cannot escape through paths or symlinks", (t) => {
	const f = overrideFixture(t);
	const outside = join(f.directory, "outside");
	mkdirSync(outside);
	writeFileSync(
		join(outside, "Cargo.toml"),
		'[package]\nname = "craft-audio"\nversion = "0.1.0"\n',
	);
	symlinkSync(outside, join(f.source, "escape"));
	for (const path of ["../outside", outside, "escape", "."]) {
		assert.throws(
			() =>
				assertPinnedLockPackages(f.original, f.reconciled, {
					source: f.source,
					pinnedSourceOverrides: [{ ...f.override, path }],
				}),
			/inside the prepared source/,
		);
	}
	rmSync(f.manifest);
	symlinkSync(join(outside, "Cargo.toml"), f.manifest);
	assert.throws(
		() =>
			assertPinnedLockPackages(f.original, f.reconciled, {
				source: f.source,
				pinnedSourceOverrides: [f.override],
			}),
		/manifest must stay inside/,
	);
});

test("local manifests must explicitly declare the pinned package name and version", (t) => {
	const f = overrideFixture(t);
	for (const manifest of [
		'[package]\nname = "other"\nversion = "0.1.0"\n',
		'[package]\nname = "craft-audio"\nversion = "0.2.0"\n',
		'[package]\nname = "craft-audio"\nversion.workspace = true\n',
		'[package]\nname = "craft-audio"\nversion = "0.1.0"\nversion = "0.1.0"\n',
		'[dependencies]\nname = "craft-audio"\nversion = "0.1.0"\n',
	]) {
		writeFileSync(f.manifest, manifest);
		assert.throws(
			() =>
				assertPinnedLockPackages(f.original, f.reconciled, {
					source: f.source,
					pinnedSourceOverrides: [f.override],
				}),
			/manifest must declare/,
		);
	}
});

test("PCM reading is allowed only for a validated local audio package", (t) => {
	const f = overrideFixture(t);
	const local = assertPinnedLockPackages(f.original, f.reconciled, {
		source: f.source,
		pinnedSourceOverrides: [f.override],
	});
	const root = "soundcraft-web v0.20.1 features=[]";
	const audio = `craft-audio v0.1.0 (${local[0].path}) features=[pcm-read,lossless-encode,peaks,serde]`;
	assert.doesNotThrow(() =>
		assertPlatformMediaDependencies(
			"soundcraft",
			`${root}\n${audio}\n${audio} (*)`,
			"soundcraft-web",
			local,
		),
	);
	assert.throws(
		() =>
			assertPlatformMediaDependencies(
				"soundcraft",
				`${root}\n${audio}`,
				"soundcraft-web",
			),
		/shared media features/,
	);
	for (const changed of [
		audio.replace(local[0].path, "https://github.com/storytold/craft-libs"),
		audio.replace(local[0].path, `${local[0].path}-other`),
		audio.replace("v0.1.0", "v0.1.1"),
		audio.replace("pcm-read,", "pcm-read,decode-native,"),
	])
		assert.throws(
			() =>
				assertPlatformMediaDependencies(
					"soundcraft",
					`${root}\n${changed}`,
					"soundcraft-web",
					local,
				),
			/shared media features/,
		);
});
