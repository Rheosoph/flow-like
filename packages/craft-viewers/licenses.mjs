import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	cpSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	readdirSync,
	realpathSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const directory = dirname(fileURLToPath(import.meta.url));
const dataDirectory = join(directory, "license-data");
const target = "wasm32-unknown-unknown";
const noticeName =
	/(^|[-_.])(licen[cs]e|notice|copying|copyright|ofl|ufl|attribution|unlicense)([-_.]|$)/i;

// Unknown license combinations require review before entering a release.
const choices = new Map([
	["MIT", "MIT"],
	["Apache-2.0", "Apache-2.0"],
	["MIT OR Apache-2.0", "MIT"],
	["Apache-2.0 OR MIT", "MIT"],
	["MIT/Apache-2.0", "MIT"],
	["Apache-2.0/MIT", "MIT"],
	["Apache-2.0 / MIT", "MIT"],
	["Apache-2.0 OR GPL-2.0-only", "Apache-2.0"],
	["Apache-2.0 OR BSL-1.0", "Apache-2.0"],
	["BSD-2-Clause", "BSD-2-Clause"],
	["BSD-3-Clause", "BSD-3-Clause"],
	["BSD-3-Clause AND MIT", "BSD-3-Clause AND MIT"],
	["BSD-3-Clause OR Apache-2.0", "BSD-3-Clause"],
	["BSD-3-Clause/MIT", "MIT"],
	["BSD-3-Clause OR MIT OR Apache-2.0", "MIT"],
	["BSD-2-Clause OR Apache-2.0 OR MIT", "MIT"],
	["Zlib", "Zlib"],
	["Zlib OR Apache-2.0 OR MIT", "MIT"],
	["MIT OR Zlib OR Apache-2.0", "MIT"],
	["MIT OR Apache-2.0 OR Zlib", "MIT"],
	["0BSD OR MIT OR Apache-2.0", "MIT"],
	["Unlicense OR MIT", "MIT"],
	["CC0-1.0 OR MIT-0 OR Apache-2.0", "Apache-2.0"],
	["CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception", "Apache-2.0"],
	["Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT", "MIT"],
	["Unicode-3.0", "Unicode-3.0"],
	["(MIT OR Apache-2.0) AND Unicode-3.0", "MIT AND Unicode-3.0"],
	["(Apache-2.0 OR MIT) AND BSD-3-Clause", "MIT AND BSD-3-Clause"],
	["(MIT OR Apache-2.0) AND IJG", "MIT AND IJG"],
	[
		"(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0",
		"MIT AND OFL-1.1 AND Ubuntu-font-1.0",
	],
	["MPL-2.0", "MPL-2.0"],
]);

export function selectLicense(expression) {
	const selected = choices.get(expression);
	if (!selected)
		throw new Error(
			`Unreviewed editor dependency license: ${expression || "missing"}. Review its terms before upgrading.`,
		);
	return selected;
}

function filesBelow(base) {
	const files = [];
	for (const entry of readdirSync(base, { withFileTypes: true })) {
		if ([".git", "target", "node_modules"].includes(entry.name)) continue;
		if (entry.isDirectory()) files.push(...filesBelow(join(base, entry.name)));
		else if (entry.isFile()) files.push(join(base, entry.name));
	}
	return files.sort();
}

export function hashLicenseInputs(hash) {
	for (const path of [
		fileURLToPath(import.meta.url),
		...filesBelow(dataDirectory),
	])
		hash.update(relative(directory, path)).update(readFileSync(path));
}

export function assertNoBrandArtwork(source, wasmBytes) {
	const brand = join(source, "docs/brand");
	if (!existsSync(brand)) return;
	for (const path of filesBelow(brand).filter((path) =>
		/\.(svg|png)$/i.test(path),
	)) {
		const asset = readFileSync(path);
		const paths = /\.svg$/i.test(path)
			? [...asset.toString("utf8").matchAll(/\bd\s*=\s*(["'])(.*?)\1/g)]
					.map((match) => match[2])
					.filter((value) => value.length >= 80)
			: [];
		if (
			wasmBytes.includes(asset) ||
			paths.some((value) => wasmBytes.includes(Buffer.from(value)))
		)
			throw new Error(
				`Restricted brand artwork remains in the editor binary: ${basename(path)}. Remove its asset loader or drawing geometry.`,
			);
	}
}

export function parseDependencyTree(tree) {
	const packages = new Map();
	for (const raw of tree.trim().split("\n")) {
		const [identity, license, repository = ""] = raw
			.replace(/ \(\*\)$/, "")
			.split("\t");
		const match = identity.match(
			/^([\w-]+) v([\w.+-]+)(?: \(proc-macro\))?(?: \((.*?)\))?(?: \(proc-macro\))?$/,
		);
		if (!match)
			throw new Error(`Cannot identify editor dependency: ${identity}`);
		const [, name, version, location] = match;
		const key = `${name}@${version}`;
		const entry = {
			name,
			version,
			license,
			selectedLicense: selectLicense(license),
			repository,
			location: location === "proc-macro" ? undefined : location,
		};
		if (
			packages.has(key) &&
			JSON.stringify(packages.get(key)) !== JSON.stringify(entry)
		)
			throw new Error(
				`Conflicting dependency metadata for ${key}; review license packaging.`,
			);
		packages.set(key, entry);
	}
	return [...packages.values()].sort((a, b) =>
		`${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`),
	);
}

// Cargo generates these string fields without multiline TOML values.
function cargoString(text, key) {
	const match = text.match(
		new RegExp(`^${key} = ("(?:[^"\\\\]|\\\\.)*")$`, "m"),
	);
	return match ? JSON.parse(match[1]) : undefined;
}

function lockedPackages(source) {
	return readFileSync(join(source, "Cargo.lock"), "utf8")
		.split("[[package]]")
		.slice(1)
		.map((text) => ({
			name: cargoString(text, "name"),
			version: cargoString(text, "version"),
			source: cargoString(text, "source"),
			checksum: cargoString(text, "checksum"),
		}));
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const portable = (path) => path.split(sep).join("/");
const escapeHtml = (value) =>
	String(value).replace(
		/[&<>"']/g,
		(c) =>
			({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
				c
			],
	);
const href = (path) => path.split("/").map(encodeURIComponent).join("/");

function registryPackage(pkg, lock, cargoHome) {
	const matches = lock.filter(
		(item) =>
			item.name === pkg.name &&
			item.version === pkg.version &&
			item.source?.startsWith("registry+"),
	);
	if (matches.length !== 1)
		throw new Error(
			`Cannot identify locked registry source for ${pkg.name} ${pkg.version}.`,
		);
	const entry = matches[0];
	const name = `${pkg.name}-${pkg.version}`;
	for (const registry of readdirSync(join(cargoHome, "registry/src"))) {
		const base = join(cargoHome, "registry/src", registry, name);
		const archive = join(
			cargoHome,
			"registry/cache",
			registry,
			`${name}.crate`,
		);
		if (
			existsSync(base) &&
			existsSync(archive) &&
			sha256(readFileSync(archive)) === entry.checksum
		)
			return { base, archive, source: entry.source, checksum: entry.checksum };
	}
	throw new Error(
		`Missing checksum-verified source archive for ${name}; restore the Cargo cache before packaging licenses.`,
	);
}

function gitPackage(pkg, lock, cargoHome, run) {
	const matches = lock.filter(
		(item) =>
			item.name === pkg.name &&
			item.version === pkg.version &&
			item.source?.startsWith("git+") &&
			item.source.slice(4).split("#")[0] === pkg.location.split("#")[0],
	);
	if (matches.length !== 1)
		throw new Error(`Cannot identify locked Git source for ${pkg.name}.`);
	const source = matches[0].source;
	const revision = source.split("#")[1];
	for (const repository of readdirSync(join(cargoHome, "git/checkouts"))) {
		for (const checkout of readdirSync(
			join(cargoHome, "git/checkouts", repository),
		)) {
			if (!revision.startsWith(checkout)) continue;
			const base = join(cargoHome, "git/checkouts", repository, checkout);
			if (run("git", ["rev-parse", "HEAD"], { cwd: base }).trim() === revision)
				return { base, source };
		}
	}
	throw new Error(`Missing pinned Git checkout for ${pkg.name}.`);
}

export function verifyUnmodifiedSource(base, archive, expectedChecksum) {
	if (sha256(readFileSync(archive)) !== expectedChecksum)
		throw new Error(
			`MPL source archive checksum mismatch: ${basename(archive)}`,
		);
	const temporary = mkdtempSync(join(tmpdir(), "craft-mpl-"));
	try {
		execFileSync("tar", ["-xzf", archive, "-C", temporary]);
		const extracted = join(temporary, basename(base));
		const ignored = new Set([".cargo-ok", ".cargo-checksum.json"]);
		const local = filesBelow(base)
			.map((path) => relative(base, path))
			.filter((path) => !ignored.has(path));
		const original = filesBelow(extracted).map((path) =>
			relative(extracted, path),
		);
		if (
			JSON.stringify(local) !== JSON.stringify(original) ||
			original.some(
				(path) =>
					!readFileSync(join(base, path)).equals(
						readFileSync(join(extracted, path)),
					),
			)
		)
			throw new Error(
				`MPL dependency ${basename(base)} has local changes. Package its modified source before distributing this build.`,
			);
	} finally {
		rmSync(temporary, { recursive: true, force: true });
	}
}

export function bundleLicenses({
	source,
	output,
	pin,
	cargoPackage,
	cargoArgs = [],
	env = process.env,
}) {
	const run = (command, args, options = {}) =>
		execFileSync(command, args, {
			cwd: source,
			env,
			encoding: "utf8",
			stdio: ["ignore", "pipe", "inherit"],
			maxBuffer: 32 * 1024 * 1024,
			...options,
		});
	const packages = parseDependencyTree(
		run("cargo", [
			"tree",
			"--offline",
			"--locked",
			"--target",
			target,
			"-p",
			cargoPackage,
			"--edges",
			"normal",
			"--prefix",
			"none",
			"--format",
			"{p}\t{l}\t{r}",
			...cargoArgs,
		]),
	);
	const licenses = join(output, "licenses");
	mkdirSync(licenses, { recursive: true });
	const upstreamFiles = filesBelow(licenses).map((path) =>
		portable(relative(licenses, path)),
	);
	const overrides = JSON.parse(
		readFileSync(join(dataDirectory, "overrides.json"), "utf8"),
	);
	const lock = lockedPackages(source);
	const cargoHome = env.CARGO_HOME || join(homedir(), ".cargo");
	const workspace = realpathSync(source);
	const put = (path, bytes) => {
		mkdirSync(dirname(join(licenses, path)), { recursive: true });
		writeFileSync(join(licenses, path), bytes);
		return path;
	};
	for (const pkg of packages) {
		const key = `${pkg.name}@${pkg.version}`;
		const prefix = `dependencies/${pkg.name}-${pkg.version}`;
		const resolved =
			!pkg.location || /^https?:/.test(pkg.location)
				? pkg.location
					? gitPackage(pkg, lock, cargoHome, run)
					: registryPackage(pkg, lock, cargoHome)
				: undefined;
		const revision = resolved
			? resolved.archive
				? existsSync(join(resolved.base, ".cargo_vcs_info.json"))
					? JSON.parse(
							readFileSync(join(resolved.base, ".cargo_vcs_info.json"), "utf8"),
						).git?.sha1
					: undefined
				: resolved.source.split("#")[1]
			: undefined;
		const override = selectNoticeOverride(overrides, key, revision);
		const additional = (override?.additionalLicenses || []).map(selectLicense);
		pkg.selectedLicense = [
			...new Set([pkg.selectedLicense, ...additional]),
		].join(" AND ");
		if (override?.obligations) pkg.obligations = override.obligations;
		const requiresSource = pkg.selectedLicense
			.split(" AND ")
			.includes("MPL-2.0");
		pkg.notices = [];
		if (pkg.location && !/^https?:/.test(pkg.location)) {
			if (requiresSource)
				throw new Error(
					`Package the exact modified/path MPL source for ${key} before distributing it.`,
				);
			const location = realpathSync(pkg.location);
			if (location !== workspace && !location.startsWith(`${workspace}${sep}`))
				throw new Error(
					`Unreviewed path dependency ${pkg.name}: ${pkg.location}`,
				);
			pkg.source = `${pin.repository.replace(/\.git$/, "")}/tree/${pin.revision}/${portable(relative(workspace, location))}`;
			pkg.notices = upstreamFiles;
		} else {
			pkg.source = resolved.source;
			pkg.checksum = resolved.checksum;
			for (const path of filesBelow(resolved.base).filter((path) =>
				noticeName.test(basename(path)),
			)) {
				pkg.notices.push(
					put(
						`${prefix}/${portable(relative(resolved.base, path))}.txt`,
						readFileSync(path),
					),
				);
			}
			if (
				!override &&
				Object.keys(overrides).some((name) => name.startsWith(`${pkg.name}@`))
			)
				throw new Error(
					`Review the missing-notice override for the new ${key} version.`,
				);
			if (override) {
				if (revision !== override.revision)
					throw new Error(
						`Review the changed upstream source for ${key} before reusing its license notices.`,
					);
			}
			for (const path of override?.files || []) {
				if (
					path.includes("..") ||
					resolve(dataDirectory, path) === dataDirectory ||
					!resolve(dataDirectory, path).startsWith(`${dataDirectory}${sep}`)
				)
					throw new Error(`Invalid license override path: ${path}`);
				pkg.notices.push(
					put(
						`${prefix}/upstream/${path}`,
						readFileSync(join(dataDirectory, path)),
					),
				);
			}
			if (override) pkg.noticeSources = override.sources;
			if (pkg.name === "epaint_default_fonts") {
				const fontBase = resolved.archive
					? resolved.base
					: join(resolved.base, "crates/epaint_default_fonts");
				for (const font of [
					"UFL.txt",
					"OFL.txt",
					"Hack-Regular.txt",
					"emoji-icon-font-mit-license.txt",
				]) {
					const path = `fonts/${font}`;
					const destination = `${prefix}/${path}.txt`;
					if (!pkg.notices.includes(destination))
						pkg.notices.push(
							put(destination, readFileSync(join(fontBase, path))),
						);
				}
			}
			if (pkg.name === "jpeg-encoder") {
				const header = readFileSync(
					join(resolved.base, "src/fdct.rs"),
					"utf8",
				).match(/^\/\*[\s\S]*?\*\//)?.[0];
				if (!header?.includes("Independent JPEG Group"))
					throw new Error("Review jpeg-encoder's changed IJG notice.");
				pkg.notices.push(
					put(
						`${prefix}/IJG.txt`,
						`${header}\n\nThis software is based in part on the work of the Independent JPEG Group.\n`,
					),
				);
			}
			if (!pkg.notices.length)
				throw new Error(
					`No license text for ${key}; add its pinned upstream notices to license-data/overrides.json.`,
				);
			if (requiresSource) {
				mkdirSync(join(licenses, "sources"), { recursive: true });
				if (resolved.archive) {
					verifyUnmodifiedSource(
						resolved.base,
						resolved.archive,
						resolved.checksum,
					);
					pkg.sourceArchive = `sources/${pkg.name}-${pkg.version}.crate`;
					cpSync(resolved.archive, join(licenses, pkg.sourceArchive));
				} else {
					pkg.sourceArchive = `sources/${pkg.name}-${pkg.version}-${revision.slice(0, 12)}.tar.gz`;
					archiveGitSource(
						resolved.base,
						revision,
						join(licenses, pkg.sourceArchive),
					);
				}
			}
		}
		pkg.location = undefined;
	}
	const manifest = {
		target,
		package: cargoPackage,
		revision: pin.revision,
		upstreamFiles,
		packages,
	};
	put("manifest.json", `${JSON.stringify(manifest, null, 2)}\n`);
	const renderNotices = (paths) =>
		paths
			.map(
				(path) =>
					`<li><a href="${href(path)}">${escapeHtml(path.split("/").slice(-2).join("/"))}</a></li>`,
			)
			.join("");
	const mpl = packages.filter((pkg) => pkg.sourceArchive);
	put(
		"index.html",
		`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'">
<title>${escapeHtml(pin.label || "Flows Word")} licenses and source</title>
<style>:root{color-scheme:light dark;font:15px system-ui,sans-serif}body{max-width:72rem;margin:0 auto;padding:24px;line-height:1.5}a{color:light-dark(#154fc1,#9ac1ff)}h1{font-size:1.5rem}h2{font-size:1.2rem}details{border-top:1px solid #8886;padding:12px 0}summary{cursor:pointer}li{overflow-wrap:anywhere}pre{white-space:pre-wrap;overflow-wrap:anywhere;font-size:13px}code{overflow-wrap:anywhere}</style></head><body>
<h1>${escapeHtml(pin.label || "Flows Word")}: third-party licenses and source</h1>
<p>This editor includes open-source software and other third-party materials. Their own licenses apply to those materials. Flow-Like's license and usage restrictions do not limit the rights those licenses grant, including rights to MPL-covered source.</p>
<p>Based on ${escapeHtml(pin.repository.replace(/\.git$/, ""))}, revision <code>${escapeHtml(pin.revision)}</code>. The upstream application code is used under MIT. Dependencies may have different terms, listed below. Warranty or support offered by Flow-Like is provided by Rheosoph GmbH alone, not the upstream contributors.</p>
${mpl.length ? `<h2>MPL source downloads</h2><p>These unmodified dependencies contain code covered by the Mozilla Public License 2.0. Download their exact source archives below. Each .crate or .tar.gz file is a gzip-compressed tar archive containing the source, including MPL-covered files. The full MPL license accompanies each dependency's notices. Covered files remain under MPL-2.0; other files retain their stated licenses.</p><ul>${mpl.map((pkg) => `<li><a download href="${href(pkg.sourceArchive)}">${escapeHtml(pkg.name)} ${escapeHtml(pkg.version)} source</a> (${escapeHtml(pkg.selectedLicense)})</li>`).join("")}</ul>` : ""}
${packages.some((pkg) => pkg.selectedLicense.includes("IJG")) ? "<p>This software is based in part on the work of the Independent JPEG Group.</p>" : ""}
<h2>Application and asset notices</h2><ul>${renderNotices(upstreamFiles)}</ul>
<h2>Dependency notices</h2><p>The list follows this editor's ${target} dependency graph. <a href="manifest.json">Download the dependency inventory</a>.</p>
${packages
	.map(
		(pkg) =>
			`<details><summary>${escapeHtml(pkg.name)} ${escapeHtml(pkg.version)} · ${escapeHtml(pkg.selectedLicense)}</summary><p>Declared license: ${escapeHtml(pkg.license)}</p>${pkg.obligations ? `<p>${escapeHtml(pkg.obligations)}</p>` : ""}<ul>${renderNotices(pkg.notices)}</ul>${pkg.notices
				.filter((path) => !upstreamFiles.includes(path))
				.map(
					(path) =>
						`<details><summary>${escapeHtml(basename(path))}</summary><pre>${escapeHtml(readFileSync(join(licenses, path), "utf8"))}</pre></details>`,
				)
				.join("")}</details>`,
	)
	.join("\n")}
</body></html>\n`,
	);
	console.log(
		`${pin.label || "Flows Word"}: packaged ${packages.length} dependency notices and ${mpl.length} MPL source archives.`,
	);
	return manifest;
}

export function selectNoticeOverride(overrides, key, revision) {
	return (revision && overrides[`${key}#${revision}`]) || overrides[key];
}

export function archiveGitSource(base, revision, destination) {
	const git = (args, options = {}) =>
		execFileSync("git", args, {
			cwd: base,
			encoding: "utf8",
			maxBuffer: 64 * 1024 * 1024,
			...options,
		});
	if (
		!/^[a-f0-9]{40}$/.test(revision) ||
		git(["rev-parse", "HEAD"]).trim() !== revision
	)
		throw new Error(
			"The dependency source no longer matches its pinned Git revision.",
		);
	const changes = git(["status", "--porcelain", "--untracked-files=all"])
		.split("\n")
		.filter((line) => line && line !== "?? .cargo-ok");
	if (changes.length)
		throw new Error(
			"The Git dependency has modified source; review it before packaging.",
		);
	writeFileSync(
		destination,
		git(
			["archive", "--format=tar.gz", `--prefix=source-${revision}/`, revision],
			{ encoding: undefined },
		),
	);
}
