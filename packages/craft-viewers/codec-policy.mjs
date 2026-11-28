import { readFileSync, realpathSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";

const platformMediaEditors = new Set(["soundcraft", "deckcraft", "filmcraft"]);

// These adapters retain the pinned PCM readers and replace compressed decoding
// with browser codecs. AAF changes only its audio feature to use those readers.
const localMediaRevisions = new Set([
	"a3fca1707941bce4768586661ccbb94c44ee46eb",
	"33be35625e252cc4f06aa375b3b428521faea37d",
]);

// The beta compiler needs the reviewed inventory constructor selector patch.
// Its source is copied from this Git revision over the exact registry release.
const inventorySource =
	"git+https://github.com/dtolnay/inventory?rev=ffc228473d7aa20ca0790ddccc894a20682489ae#ffc228473d7aa20ca0790ddccc894a20682489ae";
const inventoryIdentity = JSON.stringify([
	"inventory",
	"0.3.25",
	"registry+https://github.com/rust-lang/crates.io-index",
	"6928282826c822ad91bf1c9a1cb90a30ba1c26770749929b4656cd6be829cd7c",
]);

function localPackageOverrides(original, source, overrides) {
	if (!Array.isArray(overrides))
		throw new Error("Pinned source overrides must be an array.");
	const seen = new Set();
	return overrides.map((override) => {
		const { name, version, source: gitSource, path } = override || {};
		const revision = gitSource?.match(
			/^git\+https:\/\/github\.com\/storytold\/craft-libs\?rev=([a-f0-9]{40})#\1$/,
		)?.[1];
		const mediaOverride =
			new Set(["craft-audio", "craft-aaf"]).has(name) &&
			version === "0.1.0" &&
			localMediaRevisions.has(revision) &&
			original.has(JSON.stringify([name, version, gitSource, null]));
		const inventoryOverride =
			name === "inventory" &&
			version === "0.3.25" &&
			gitSource === inventorySource &&
			original.has(inventoryIdentity);
		if ((!mediaOverride && !inventoryOverride) || seen.has(name))
			throw new Error(
				`Unreviewed pinned source override: ${JSON.stringify(override)}.`,
			);
		seen.add(name);
		if (!source || typeof path !== "string" || !path || isAbsolute(path))
			throw new Error(
				`The ${name} override must stay inside the prepared source.`,
			);
		const root = realpathSync(source);
		const local = resolve(root, path);
		const inside = (candidate) => {
			const child = relative(root, candidate);
			return (
				child &&
				child !== ".." &&
				!child.startsWith(`..${sep}`) &&
				!isAbsolute(child)
			);
		};
		if (!inside(local) || !inside(realpathSync(local)))
			throw new Error(
				`The ${name} override must stay inside the prepared source.`,
			);
		const manifest = realpathSync(join(local, "Cargo.toml"));
		if (!inside(manifest))
			throw new Error(
				`The ${name} manifest must stay inside the prepared source.`,
			);
		const packageSection = readFileSync(manifest, "utf8").match(
			/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m,
		)?.[1];
		for (const [field, expected] of [
			["name", name],
			["version", version],
		]) {
			const entries = [
				...(packageSection || "").matchAll(
					new RegExp(`^${field} = ("[^"\\n]*")\\s*$`, "gm"),
				),
			];
			if (entries.length !== 1 || JSON.parse(entries[0][1]) !== expected)
				throw new Error(
					`The ${name} override manifest must declare its pinned ${field}.`,
				);
		}
		return { name, version, source: gitSource, path: realpathSync(local) };
	});
}

// Containers, PCM and lossless file writers remain in Rust. Compressed audio
// and video are handled by browser/OS APIs in these three editor adapters.
const softwareCodec =
	/(?:^|[-_])(?:aac|ac3|eac3|h264|h264enc|h265|hevc|av1|apv|dnx|mpeg2v|mpa|mp2|mp3|vorbis|alac|opus|prores|vp8|vp9|rav1e|dav1d|openh264|ffmpeg|x264|x265|libavcodec|libvpx|minimp3|puremp3|rmp3|lewton|audiopus|mp3lame|lame)(?:[-_]|$)/;

// Shared crates can hide codecs behind features instead of separate packages.
// New features need review before entering a browser media build.
const sharedMediaFeatures = new Map([
	[
		"craft-audio",
		new Set([
			"default",
			"pcm-encode",
			"flac-encode",
			"lossless-encode",
			"peaks",
			"serde",
			"rayon",
		]),
	],
	["craft-video", new Set(["default", "threads", "rayon"])],
]);

export function usesPlatformMedia(id) {
	return platformMediaEditors.has(id);
}

function lockedPackages(lock) {
	return lock
		.split("[[package]]")
		.slice(1)
		.map((block) => {
			const fields = ["name", "version", "source", "checksum"].map((field) => {
				const value = block.match(new RegExp(`^${field} = (".*")$`, "m"));
				return value ? JSON.parse(value[1]) : null;
			});
			if (!fields[0] || !fields[1])
				throw new Error(
					"Cannot verify the editor's Cargo.lock package identity.",
				);
			return JSON.stringify(fields);
		});
}

export function assertPinnedLockPackages(
	original,
	reconciled,
	{ source, pinnedSourceOverrides = [] } = {},
) {
	const pinned = new Set(lockedPackages(original));
	const resolved = lockedPackages(reconciled);
	if (!pinned.size || !resolved.length)
		throw new Error("Cannot verify an empty editor Cargo.lock.");
	const local = localPackageOverrides(pinned, source, pinnedSourceOverrides);
	for (const { name, version } of local) {
		const identity = JSON.stringify([name, version, null, null]);
		if (resolved.filter((pkg) => pkg === identity).length !== 1)
			throw new Error(
				`The reconciled lockfile must contain the local ${name} override exactly once.`,
			);
		pinned.add(identity);
	}
	for (const pkg of resolved)
		if (!pinned.has(pkg))
			throw new Error(
				`The codec adapter resolved a package outside its pinned lockfile: ${pkg}. Review and pin the dependency before building.`,
			);
	return local;
}

export function assertPlatformMediaDependencies(
	id,
	tree,
	cargoPackage,
	localPackages = [],
) {
	if (!usesPlatformMedia(id)) return;
	const packages = new Set();
	const unreviewed = new Set();
	for (const line of tree.trim().split("\n")) {
		const match = line.match(/^([\w-]+) v[\w.+-]+(?:\s|$)/);
		if (!match) throw new Error(`Cannot check ${id} codec dependency: ${line}`);
		packages.add(match[1]);
		const allowed = sharedMediaFeatures.get(match[1]);
		if (allowed) {
			const features = line.match(/ features=\[([^\]]*)\](?: \(\*\))?$/);
			if (!features)
				throw new Error(`Cannot check ${id} activated features: ${line}`);
			const pcmReader =
				match[1] === "craft-audio" &&
				localPackages.some(
					(pkg) =>
						pkg.name === match[1] &&
						line.startsWith(
							`${pkg.name} v${pkg.version} (${pkg.path}) features=[`,
						),
				);
			for (const feature of features[1].split(",").filter(Boolean))
				if (!allowed.has(feature) && !(feature === "pcm-read" && pcmReader))
					unreviewed.add(`${match[1]}/${feature}`);
		}
	}
	if (!packages.has(cargoPackage))
		throw new Error(`The codec dependency graph is missing ${cargoPackage}.`);
	const bundled = [...packages].filter(
		(name) => /^symphonia(?:-|$)/.test(name) || softwareCodec.test(name),
	);
	if (bundled.length)
		throw new Error(
			`${id} must use browser/OS media codecs. Remove bundled software codec dependencies before distributing it: ${bundled.sort().join(", ")}`,
		);
	if (unreviewed.size)
		throw new Error(
			`${id} must use browser/OS media codecs. Remove or review shared media features before distributing it: ${[...unreviewed].sort().join(", ")}`,
		);
}
