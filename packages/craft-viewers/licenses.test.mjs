import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	chmodSync,
	cpSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import test from "node:test";
import {
	archiveGitSource,
	assertNoBrandArtwork,
	bundleLicenses,
	chunkSourceArchive,
	parseDependencyTree,
	renderLicenseIndex,
	selectLicense,
	selectNoticeOverride,
	verifyUnmodifiedSource,
} from "./licenses.mjs";

const checksum = (bytes) => createHash("sha256").update(bytes).digest("hex");
const put = (path, text) => {
	mkdirSync(dirname(path), { recursive: true });
	writeFileSync(path, text);
};

function sourceFixture(t, name = "test-source", version = "1.0.0", files = {}) {
	const directory = mkdtempSync(join(tmpdir(), "craft-license-test-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	const original = join(directory, "original", `${name}-${version}`);
	put(
		join(original, "Cargo.toml"),
		`[package]\nname = "${name}"\nversion = "${version}"\n`,
	);
	put(join(original, "src/lib.rs"), "pub fn answer() -> u8 { 42 }\n");
	put(
		join(original, "LICENSE"),
		"Fixture license notice: <script>alert('notice')</script>\n<SCRIPT >alert('upper')</SCRIPT >\n<ScRiPt/x>alert('slash')</ScRiPt>\n",
	);
	for (const [path, text] of Object.entries(files))
		put(join(original, path), text);
	const archive = join(directory, `${name}-${version}.crate`);
	execFileSync("tar", [
		"-czf",
		archive,
		"-C",
		dirname(original),
		`${name}-${version}`,
	]);
	const base = join(directory, "extracted", `${name}-${version}`);
	cpSync(original, base, { recursive: true });
	return {
		directory,
		original,
		base,
		archive,
		hash: checksum(readFileSync(archive)),
	};
}

function bundleFixture(
	t,
	{
		name = "symphonia-core",
		version = "0.5.5",
		license = "MPL-2.0",
		revision = "6d533f26150953a882a6a111ebd13f0abf7129d5",
		path = "symphonia-core",
		files = {},
	} = {},
) {
	const fixture = sourceFixture(t, name, version, {
		".cargo_vcs_info.json": JSON.stringify({
			git: { sha1: revision },
			path_in_vcs: path,
		}),
		...files,
	});
	const source = join(fixture.directory, "workspace");
	const output = join(fixture.directory, "output");
	const cargoHome = join(fixture.directory, "cargo-home");
	const registry = "index.crates.io-fixture";
	const base = join(cargoHome, "registry/src", registry, `${name}-${version}`);
	cpSync(fixture.base, base, { recursive: true });
	put(
		join(cargoHome, "registry/cache", registry, `${name}-${version}.crate`),
		readFileSync(fixture.archive),
	);
	put(
		join(source, "Cargo.lock"),
		`version = 4\n\n[[package]]\nname = "${name}"\nversion = "${version}"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "${fixture.hash}"\n`,
	);
	put(join(output, "licenses/LICENSE-MIT"), "Application fixture license\n");
	const bin = join(fixture.directory, "bin");
	const cargo = join(bin, "cargo");
	put(
		cargo,
		`#!${process.execPath}\nprocess.stdout.write(process.env.CRAFT_TEST_DEPENDENCIES);\n`,
	);
	chmodSync(cargo, 0o755);
	const tree = `test-editor v1.0.0 (${source})\tMIT\thttps://example.com/editor\n${name} v${version}\t${license}\thttps://example.com/${name}\n`;
	const env = {
		...process.env,
		PATH: `${bin}${delimiter}${process.env.PATH || ""}`,
		CARGO_HOME: cargoHome,
		CRAFT_TEST_DEPENDENCIES: tree,
	};
	const args = {
		source,
		output,
		cargoPackage: "test-editor",
		pin: {
			repository: "https://example.com/editor.git",
			revision: "a".repeat(40),
			label: "Editor <test>",
		},
		env,
	};
	return { ...fixture, base, source, output, env, args };
}

test("license choices preserve required additional terms and choose permissive alternatives", () => {
	assert.equal(selectLicense("Apache-2.0 OR GPL-2.0-only"), "Apache-2.0");
	assert.equal(selectLicense("MIT OR Apache-2.0"), "MIT");
	assert.equal(selectLicense("MPL-2.0"), "MPL-2.0");
	assert.equal(selectLicense("CC-BY-4.0"), "CC-BY-4.0");
	assert.equal(selectLicense("(MIT OR Apache-2.0) AND IJG"), "MIT AND IJG");
	assert.equal(
		selectLicense("(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0"),
		"MIT AND OFL-1.1 AND Ubuntu-font-1.0",
	);
});

test("unreviewed, missing, and mandatory strong copyleft licenses fail closed", () => {
	for (const expression of [
		undefined,
		"",
		"GPL-3.0-only",
		"AGPL-3.0-only",
		"LGPL-2.1-only",
		"LicenseRef-Custom",
		"MIT AND GPL-2.0-only",
	])
		assert.throws(
			() => selectLicense(expression),
			/Unreviewed editor dependency license/,
		);
});

test("dependency parsing handles repeated, path, Git, prerelease, and proc-macro packages", () => {
	const tree = [
		"fixture v1.0.0 (/tmp/editor workspace)\tMIT\thttps://example.com/editor",
		"derive v0.2.0 (proc-macro)\tMIT\thttps://example.com/derive",
		"remote v1.2.3-beta.1 (https://example.com/lib?rev=abc#abcdef)\tBSD-3-Clause\thttps://example.com/lib",
		"derive v0.2.0 (proc-macro)\tMIT\thttps://example.com/derive (*)",
	].join("\n");
	const packages = parseDependencyTree(tree);
	assert.equal(packages.length, 3);
	assert.equal(
		packages.find((pkg) => pkg.name === "fixture").location,
		"/tmp/editor workspace",
	);
	assert.equal(
		packages.find((pkg) => pkg.name === "derive").location,
		undefined,
	);
	assert.equal(
		packages.find((pkg) => pkg.name === "remote").location,
		"https://example.com/lib?rev=abc#abcdef",
	);
});

test("local proc-macro dependencies retain their source directory", () => {
	for (const identity of [
		"solidcraft-derive v0.15.0 (proc-macro) (/tmp/editor workspace/crates/derive)",
		"solidcraft-derive v0.15.0 (/tmp/editor workspace/crates/derive) (proc-macro)",
	]) {
		const [dependency] = parseDependencyTree(
			`${identity}\tMIT OR Apache-2.0\thttps://github.com/tangency-labs/solidcraft`,
		);
		assert.equal(dependency.location, "/tmp/editor workspace/crates/derive");
		assert.equal(dependency.selectedLicense, "MIT");
	}
});

test("dependency parsing retains source variants and rejects conflicting metadata for one identity", () => {
	assert.throws(
		() => parseDependencyTree("unexpected Cargo output"),
		/Cannot identify/,
	);
	assert.equal(
		parseDependencyTree(
			"fixture v1.0.0 (/one)\tMIT\t\nfixture v1.0.0 (/two)\tApache-2.0\t",
		).length,
		2,
	);
	assert.throws(
		() =>
			parseDependencyTree(
				"fixture v1.0.0\tMIT\t\nfixture v1.0.0\tApache-2.0\t",
			),
		/conflict|metadata/i,
	);
	assert.throws(
		() =>
			parseDependencyTree(
				"fixture v1.0.0 (/same)\tMIT\thttps://one\nfixture v1.0.0 (/same)\tMIT\thttps://two",
			),
		/conflict|metadata/i,
	);
});

test("MPL source verification accepts byte-identical sources and Cargo cache markers", (t) => {
	const fixture = sourceFixture(t);
	put(join(fixture.base, ".cargo-ok"), "cache marker");
	put(join(fixture.base, ".cargo-checksum.json"), "{}");
	assert.doesNotThrow(() =>
		verifyUnmodifiedSource(fixture.base, fixture.archive, fixture.hash),
	);
});

test("MPL source verification rejects changed, removed, added files and wrong archives", (t) => {
	const fixture = sourceFixture(t);
	const verify = () =>
		verifyUnmodifiedSource(fixture.base, fixture.archive, fixture.hash);
	put(join(fixture.base, "src/lib.rs"), "pub fn answer() -> u8 { 7 }\n");
	assert.throws(verify, /local changes/);
	cpSync(fixture.original, fixture.base, { recursive: true });
	rmSync(join(fixture.base, "src/lib.rs"));
	assert.throws(verify, /local changes/);
	cpSync(fixture.original, fixture.base, { recursive: true });
	put(join(fixture.base, "src/extra.rs"), "pub const EXTRA: bool = true;\n");
	assert.throws(verify, /local changes/);
	assert.throws(
		() => verifyUnmodifiedSource(fixture.base, fixture.archive, "0".repeat(64)),
		/checksum mismatch/,
	);
});

test(
	"bundle publishes exact MPL source and links to complete notice files",
	{ skip: process.platform === "win32" },
	(t) => {
		const fixture = bundleFixture(t);
		const manifest = bundleLicenses(fixture.args);
		const pkg = manifest.packages.find(
			(item) => item.name === "symphonia-core",
		);
		assert.equal(pkg.selectedLicense, "MPL-2.0");
		assert.equal(pkg.checksum, fixture.hash);
		const source = join(fixture.output, "licenses", pkg.sourceArchive);
		assert.deepEqual(readFileSync(source), readFileSync(fixture.archive));
		assert.equal(pkg.sourceArchiveSha256, fixture.hash);
		assert.equal(pkg.sourceArchiveByteLength, readFileSync(source).length);
		assert.equal(pkg.sourceArchiveParts, undefined);
		assert.ok(
			pkg.notices.some((path) => path.endsWith("upstream/symphonia/LICENSE")),
		);
		const html = readFileSync(
			join(fixture.output, "licenses/index.html"),
			"utf8",
		);
		assert.match(html, /MPL source downloads/);
		assert.ok(html.includes(`href="${pkg.sourceArchive}"`));
		assert.match(html, /Editor &lt;test&gt;/);
		assert.doesNotMatch(html, /<script\b/i);
		assert.doesNotMatch(html, /<pre>/);
		for (const notice of pkg.notices) {
			assert.ok(html.includes(`href="${notice}"`));
		}
		const notice = pkg.notices.find((path) => path.endsWith("/LICENSE.txt"));
		assert.deepEqual(
			readFileSync(join(fixture.output, "licenses", notice)),
			readFileSync(join(fixture.base, "LICENSE")),
		);
		assert.match(
			readFileSync(
				join(
					fixture.output,
					"licenses",
					pkg.notices.find((path) => path.endsWith("/symphonia/LICENSE")),
				),
				"utf8",
			),
			/Mozilla Public License Version 2\.0/,
		);
		assert.equal(
			JSON.parse(readFileSync(join(fixture.output, "licenses/manifest.json")))
				.packages.length,
			2,
		);
	},
);

test("large dependency catalogs keep notices and source archives accessible in a compact index", () => {
	const packages = Array.from({ length: 400 }, (_, index) => ({
		name: `library-${index}`,
		version: "1.0.0",
		license: "MIT",
		selectedLicense: "MIT",
		source: "git+https://example.com/source#abc",
		notices: Array.from(
			{ length: 20 },
			(_, notice) => `dependencies/library-${index}/notice-${notice}.txt`,
		),
		...(index === 0
			? {
					sourceArchive: "sources/exact-source.tar.gz",
					obligations: "Keep <copyright> & attribution.",
				}
			: {}),
	}));
	const html = renderLicenseIndex(
		{
			target: "wasm32-unknown-unknown",
			upstreamFiles: ["LICENSE-MIT"],
			packages,
		},
		{
			label: "Editor <test>",
			repository: "https://example.com/editor.git",
			revision: "a".repeat(40),
		},
	);
	assert.ok(Buffer.byteLength(html) < 2 * 1024 * 1024);
	assert.match(html, /Keep &lt;copyright&gt; &amp; attribution\./);
	assert.match(html, /download href="sources\/exact-source.tar.gz"/);
	assert.ok(html.includes('href="LICENSE-MIT"'));
	for (const pkg of packages) {
		for (const path of pkg.notices) assert.ok(html.includes(`href="${path}"`));
	}
	assert.doesNotMatch(html, /<pre>|<script\b/i);
});

test(
	"oversized source archives split losslessly and the published commands reconstruct and verify them",
	{ skip: process.platform === "win32" },
	(t) => {
		const directory = mkdtempSync(join(tmpdir(), "craft-source-parts-"));
		t.after(() => rmSync(directory, { recursive: true, force: true }));
		const archive = "sources/source's archive.tar.gz";
		const bytes = Buffer.alloc(25 * 1024 * 1024 + 17);
		for (let offset = 0; offset < bytes.length; offset += 1024 * 1024)
			bytes.fill(
				offset / (1024 * 1024),
				offset,
				Math.min(offset + 1024 * 1024, bytes.length),
			);
		put(join(directory, archive), bytes);
		const metadata = chunkSourceArchive(directory, archive);
		assert.equal(existsSync(join(directory, archive)), false);
		assert.equal(metadata.sourceArchiveByteLength, bytes.length);
		assert.equal(metadata.sourceArchiveSha256, checksum(bytes));
		assert.equal(metadata.sourceArchiveParts.length, 4);
		const parts = metadata.sourceArchiveParts.map((part) => {
			const data = readFileSync(join(directory, part.path));
			assert.equal(data.length, part.byteLength);
			assert.ok(data.length <= 8 * 1024 * 1024);
			assert.equal(checksum(data), part.sha256);
			return data;
		});
		assert.deepEqual(Buffer.concat(parts), bytes);
		const html = renderLicenseIndex(
			{
				target: "wasm32-unknown-unknown",
				upstreamFiles: [],
				packages: [
					{
						name: "source",
						version: "1.0.0",
						license: "MPL-2.0",
						selectedLicense: "MPL-2.0",
						source: "git+https://example.com/source#abc",
						notices: [],
						sourceArchive: archive,
						...metadata,
					},
				],
			},
			{ repository: "https://example.com/editor", revision: "a".repeat(40) },
		);
		assert.ok(Buffer.byteLength(html) < 25 * 1024 * 1024);
		assert.ok(
			!html.includes(
				`href="${archive.split("/").map(encodeURIComponent).join("/")}"`,
			),
		);
		for (const part of metadata.sourceArchiveParts)
			assert.ok(
				html.includes(
					`download href="${part.path.split("/").map(encodeURIComponent).join("/")}"`,
				),
			);
		const encoded = html.match(/<pre>([\s\S]*?)<\/pre>/)?.[1];
		assert.ok(encoded);
		const entities = {
			"&amp;": "&",
			"&lt;": "<",
			"&gt;": ">",
			"&quot;": '"',
			"&#39;": "'",
		};
		const command = encoded.replace(
			/&(?:amp|lt|gt|quot|#39);/g,
			(entity) => entities[entity],
		);
		const downloaded = join(directory, "downloaded");
		mkdirSync(downloaded);
		for (const part of metadata.sourceArchiveParts)
			cpSync(
				join(directory, part.path),
				join(downloaded, part.path.split("/").at(-1)),
			);
		assert.match(
			execFileSync("sh", ["-c", command], {
				cwd: downloaded,
				encoding: "utf8",
			}),
			/: OK/,
		);
		assert.deepEqual(
			readFileSync(join(downloaded, archive.split("/").at(-1))),
			bytes,
		);
		put(
			join(downloaded, metadata.sourceArchiveParts[0].path.split("/").at(-1)),
			Buffer.from("corrupt"),
		);
		assert.throws(
			() =>
				execFileSync("sh", ["-c", command], { cwd: downloaded, stdio: "pipe" }),
			/Command failed/,
		);
	},
);

test(
	"copied MPL code adds source obligations while preserving the package's primary license",
	{ skip: process.platform === "win32" },
	(t) => {
		const fixture = bundleFixture(t, {
			name: "krilla",
			version: "0.8.2",
			license: "MIT OR Apache-2.0",
			revision: "3ffdf0588cf98050aad6edba51ca70162e1fb5b5",
			path: "crates/krilla",
			files: {
				"src/content.rs":
					"// MPL-covered fixture source\npub fn content_draw_path() {}\n",
			},
		});
		const manifest = bundleLicenses(fixture.args);
		const pkg = manifest.packages.find((item) => item.name === "krilla");
		assert.equal(pkg.license, "MIT OR Apache-2.0");
		assert.equal(pkg.selectedLicense, "MIT AND Apache-2.0 AND MPL-2.0");
		assert.match(pkg.obligations, /content_draw_path/);
		assert.deepEqual(
			readFileSync(join(fixture.output, "licenses", pkg.sourceArchive)),
			readFileSync(fixture.archive),
		);
		const html = readFileSync(
			join(fixture.output, "licenses/index.html"),
			"utf8",
		);
		assert.ok(html.includes(`href="${pkg.sourceArchive}"`));
		assert.match(html, /contain code covered by the Mozilla Public License/);
		assert.match(
			html,
			/Covered files remain under MPL-2\.0; other files retain their stated licenses/,
		);
		assert.doesNotMatch(
			html,
			/The following unmodified libraries are covered by the Mozilla Public License/,
		);
		put(
			join(fixture.base, "src/content.rs"),
			"pub fn modified_content_draw_path() {}\n",
		);
		assert.throws(() => bundleLicenses(fixture.args), /local changes/);
	},
);

test(
	"bundle refuses registry modifications instead of serving unmatched MPL source",
	{ skip: process.platform === "win32" },
	(t) => {
		const fixture = bundleFixture(t);
		put(join(fixture.base, "src/lib.rs"), "pub fn changed() {}\n");
		assert.throws(() => bundleLicenses(fixture.args), /local changes/);
	},
);

test(
	"dependency upgrades require renewed review of missing-notice overrides",
	{ skip: process.platform === "win32" },
	(t) => {
		const fixture = bundleFixture(t, { version: "0.5.6" });
		assert.throws(
			() => bundleLicenses(fixture.args),
			/Review.*override.*new symphonia-core@0\.5\.6/,
		);
	},
);

test(
	"dependency notices cannot be reused for another upstream commit at the same version",
	{ skip: process.platform === "win32" },
	(t) => {
		const fixture = bundleFixture(t);
		put(
			join(fixture.base, ".cargo_vcs_info.json"),
			JSON.stringify({ git: { sha1: "b".repeat(40) } }),
		);
		assert.throws(
			() => bundleLicenses(fixture.args),
			/changed upstream source/,
		);
	},
);

test(
	"bundle refuses MPL workspace source until its modified source is packaged",
	{ skip: process.platform === "win32" },
	(t) => {
		const fixture = bundleFixture(t);
		fixture.env.CRAFT_TEST_DEPENDENCIES = `test-editor v1.0.0 (${fixture.source})\tMPL-2.0\thttps://example.com/editor\n`;
		assert.throws(
			() => bundleLicenses(fixture.args),
			/MPL.*source|source.*MPL/i,
		);
	},
);

test("brand verification rejects complete SVG and PNG assets in a binary", (t) => {
	const directory = mkdtempSync(join(tmpdir(), "craft-brand-test-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	for (const [name, bytes] of [
		["mark.svg", Buffer.from('<svg><path d="M0 0L1 1"/></svg>')],
		["mark.png", Buffer.from([137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3, 4])],
	]) {
		put(join(directory, "docs/brand", name), bytes);
		assert.throws(
			() =>
				assertNoBrandArtwork(
					directory,
					Buffer.concat([Buffer.from("WASM"), bytes, Buffer.from("END")]),
				),
			/Restricted brand artwork remains/,
		);
	}
});

test("brand verification catches embedded SVG drawing geometry without the source SVG", (t) => {
	const directory = mkdtempSync(join(tmpdir(), "craft-brand-test-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	const geometry = `M0,0${Array.from({ length: 30 }, (_, i) => `L${i},${i + 2}`).join("")}Z`;
	for (const [name, attribute] of [
		["mark.svg", `d="${geometry}"`],
		["MARK.SVG", `d = '${geometry}'`],
	]) {
		const path = join(directory, "docs/brand", name);
		put(path, `<svg><path ${attribute}/></svg>`);
		assert.throws(
			() =>
				assertNoBrandArtwork(
					directory,
					Buffer.from(`compiled-prefix${geometry}compiled-suffix`),
				),
			/Restricted brand artwork remains/,
		);
		rmSync(path);
	}
});

test("brand verification accepts clean binaries and common short SVG paths", (t) => {
	const directory = mkdtempSync(join(tmpdir(), "craft-brand-test-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	assert.doesNotThrow(() =>
		assertNoBrandArtwork(directory, Buffer.from("no assets")),
	);
	put(
		join(directory, "docs/brand/mark.svg"),
		'<svg><path d="M0 0L1 1"/></svg>',
	);
	assert.doesNotThrow(() =>
		assertNoBrandArtwork(directory, Buffer.from("M0 0L1 1")),
	);
});

test("notice overrides distinguish forks of the same package version", () => {
	const first = { revision: "a".repeat(40) };
	const second = { revision: "b".repeat(40) };
	const overrides = {
		"library@1.0.0": first,
		[`library@1.0.0#${second.revision}`]: second,
	};
	assert.equal(
		selectNoticeOverride(overrides, "library@1.0.0", first.revision),
		first,
	);
	assert.equal(
		selectNoticeOverride(overrides, "library@1.0.0", second.revision),
		second,
	);
	assert.equal(
		selectNoticeOverride(overrides, "library@1.0.0", "c".repeat(40)),
		first,
	);
	assert.equal(
		selectNoticeOverride(overrides, "library@2.0.0", second.revision),
		undefined,
	);
});

test("Git source archives include the pinned source and refuse modified checkouts", (t) => {
	const directory = mkdtempSync(join(tmpdir(), "craft-git-license-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	const source = join(directory, "source");
	mkdirSync(source);
	const git = (...args) =>
		execFileSync("git", args, { cwd: source, encoding: "utf8" });
	git("init", "--quiet");
	put(join(source, "src/lib.rs"), "pub fn answer() -> u8 { 42 }\n");
	git("add", "src/lib.rs");
	git(
		"-c",
		"commit.gpgsign=false",
		"-c",
		"user.name=License test",
		"-c",
		"user.email=license-test@example.invalid",
		"commit",
		"--quiet",
		"-m",
		"Fixture",
	);
	const revision = git("rev-parse", "HEAD").trim();
	const output = join(directory, "source.tar.gz");
	put(join(source, ".cargo-ok"), "");
	archiveGitSource(source, revision, output);
	assert.match(
		execFileSync("tar", ["-tzf", output], { encoding: "utf8" }),
		/src\/lib.rs/,
	);
	put(join(source, "src/lib.rs"), "pub fn answer() -> u8 { 0 }\n");
	assert.throws(
		() => archiveGitSource(source, revision, output),
		/modified source/,
	);
});

test("same-version Git revisions retain their own notices and exact MPL source archives", (t) => {
	const fixture = bundleFixture(t, {
		name: "shared-library",
		version: "0.1.0",
	});
	const repository = join(fixture.directory, "git-original");
	mkdirSync(repository);
	const git = (...args) =>
		execFileSync("git", args, { cwd: repository, encoding: "utf8" }).trim();
	git("init", "--quiet");
	put(
		join(repository, "Cargo.toml"),
		'[package]\nname = "shared-library"\nversion = "0.1.0"\nlicense = "MPL-2.0"\n',
	);
	const revisions = [];
	const checkouts = [];
	for (const label of ["First", "Second"]) {
		put(join(repository, "LICENSE"), `${label} source license\n`);
		put(
			join(repository, "src/lib.rs"),
			`pub const SOURCE: &str = "${label}";\n`,
		);
		git("add", ".");
		git(
			"-c",
			"commit.gpgsign=false",
			"-c",
			"user.name=License test",
			"-c",
			"user.email=license-test@example.invalid",
			"commit",
			"--quiet",
			"-m",
			label,
		);
		const revision = git("rev-parse", "HEAD");
		const checkout = join(
			fixture.env.CARGO_HOME,
			"git/checkouts/library",
			revision.slice(0, 7),
		);
		cpSync(repository, checkout, { recursive: true });
		revisions.push(revision);
		checkouts.push(checkout);
	}
	const origin = "https://example.com/shared-library?branch=main";
	put(
		join(fixture.source, "Cargo.lock"),
		revisions
			.map(
				(revision) =>
					`[[package]]\nname = "shared-library"\nversion = "0.1.0"\nsource = "git+${origin}#${revision}"\n`,
			)
			.join("\n"),
	);
	const rows = revisions.map(
		(revision) =>
			`shared-library v0.1.0 (${origin}#${revision.slice(0, 7)})\tMPL-2.0\thttps://example.com/shared-library`,
	);
	fixture.env.CRAFT_TEST_DEPENDENCIES = `${rows.join("\n")}\n${rows[0]} (*)\n`;
	const manifest = bundleLicenses(fixture.args);
	assert.equal(manifest.packages.length, 2);
	assert.equal(new Set(manifest.packages.map((pkg) => pkg.notices[0])).size, 2);
	assert.equal(
		new Set(manifest.packages.map((pkg) => pkg.sourceArchive)).size,
		2,
	);
	for (const [index, revision] of revisions.entries()) {
		const pkg = manifest.packages.find(
			(pkg) => pkg.source === `git+${origin}#${revision}`,
		);
		const label = index === 0 ? "First" : "Second";
		assert.equal(
			readFileSync(join(fixture.output, "licenses", pkg.notices[0]), "utf8"),
			`${label} source license\n`,
		);
		const archive = join(fixture.output, "licenses", pkg.sourceArchive);
		const bytes = readFileSync(archive);
		assert.equal(pkg.sourceArchiveSha256, checksum(bytes));
		assert.equal(pkg.sourceArchiveByteLength, bytes.length);
		assert.equal(pkg.sourceArchiveParts, undefined);
		const paths = execFileSync("tar", ["-tzf", archive], { encoding: "utf8" })
			.trim()
			.split("\n");
		const lib = paths.find((path) => path.endsWith("src/lib.rs"));
		assert.equal(
			execFileSync("tar", ["-xOzf", archive, lib], { encoding: "utf8" }),
			`pub const SOURCE: &str = "${label}";\n`,
		);
	}
	put(join(checkouts[1], "src/lib.rs"), "changed source\n");
	assert.throws(() => bundleLicenses(fixture.args), /modified source/);
});

test("validated local source overrides retain original Git provenance and their exact notices", (t) => {
	const fixture = bundleFixture(t, {
		name: "craft-audio",
		version: "0.1.0",
		license: "MIT",
	});
	const local = join(fixture.source, "host-dependencies/craft-audio");
	put(
		join(local, "Cargo.toml"),
		'[package]\nname = "craft-audio"\nversion = "0.1.0"\nlicense = "MIT"\n',
	);
	put(join(local, "LICENSE-MIT"), "Pinned audio notice\n");
	put(join(local, "src/lib.rs"), "// browser PCM changes\n");
	const origin = `git+https://github.com/storytold/craft-libs?rev=${"b".repeat(40)}#${"b".repeat(40)}`;
	fixture.env.CRAFT_TEST_DEPENDENCIES = `craft-audio v0.1.0 (${local})\tMIT\thttps://github.com/storytold/craft-libs\n`;
	const args = {
		...fixture.args,
		sourceOverrides: [
			{ name: "craft-audio", version: "0.1.0", source: origin, path: local },
		],
	};
	const manifest = bundleLicenses(args);
	const pkg = manifest.packages[0];
	assert.equal(pkg.source, origin);
	assert.equal(pkg.modified, true);
	assert.equal(pkg.location, undefined);
	assert.deepEqual(pkg.notices, [
		"dependencies/craft-audio-0.1.0/LICENSE-MIT.txt",
	]);
	assert.equal(
		readFileSync(join(fixture.output, "licenses", pkg.notices[0]), "utf8"),
		"Pinned audio notice\n",
	);
	assert.match(
		readFileSync(join(fixture.output, "licenses/index.html"), "utf8"),
		/Locally modified/,
	);
	assert.throws(
		() =>
			bundleLicenses({
				...args,
				sourceOverrides: [...args.sourceOverrides, ...args.sourceOverrides],
			}),
		/Invalid validated source override/,
	);
	fixture.env.CRAFT_TEST_DEPENDENCIES =
		fixture.env.CRAFT_TEST_DEPENDENCIES.replace("\tMIT\t", "\tMPL-2.0\t");
	assert.throws(() => bundleLicenses(args), /exact modified\/path MPL source/);
});

test("crates from one Git checkout share a single copy of its notices", (t) => {
	const fixture = bundleFixture(t, { name: "craft-a", version: "0.1.0" });
	const repository = join(fixture.directory, "craft-libs");
	const git = (...args) =>
		execFileSync("git", args, { cwd: repository, encoding: "utf8" }).trim();
	put(join(repository, "LICENSE-MIT"), "Workspace notice\n");
	put(join(repository, "crates/b/NOTICE"), "Crate b notice\n");
	put(join(repository, "crates/a/src/lib.rs"), "\n");
	git("init", "--quiet");
	git("add", ".");
	git(
		"-c",
		"commit.gpgsign=false",
		"-c",
		"user.name=License test",
		"-c",
		"user.email=license-test@example.invalid",
		"commit",
		"--quiet",
		"-m",
		"workspace",
	);
	const revision = git("rev-parse", "HEAD");
	cpSync(
		repository,
		join(
			fixture.env.CARGO_HOME,
			"git/checkouts/craft-libs-fixture",
			revision.slice(0, 7),
		),
		{ recursive: true },
	);
	const origin = `https://github.com/example/craft-libs.git?rev=${revision}`;
	put(
		join(fixture.source, "Cargo.lock"),
		["craft-a", "craft-b"]
			.map(
				(name) =>
					`[[package]]\nname = "${name}"\nversion = "0.1.0"\nsource = "git+${origin}#${revision}"\n`,
			)
			.join("\n"),
	);
	fixture.env.CRAFT_TEST_DEPENDENCIES = ["craft-a", "craft-b"]
		.map(
			(name) =>
				`${name} v0.1.0 (${origin}#${revision.slice(0, 7)})\tMIT OR Apache-2.0\thttps://github.com/example/craft-libs\n`,
		)
		.join("");
	const manifest = bundleLicenses(fixture.args);
	const shared = `dependencies/git/craft-libs-${revision.slice(0, 12)}`;
	const [a, b] = manifest.packages;
	assert.deepEqual(a.notices, [
		`${shared}/LICENSE-MIT.txt`,
		`${shared}/crates/b/NOTICE.txt`,
	]);
	assert.deepEqual(b.notices, a.notices);
	assert.equal(
		readFileSync(join(fixture.output, "licenses", a.notices[1]), "utf8"),
		"Crate b notice\n",
	);
	for (const name of ["craft-a-0.1.0", "craft-b-0.1.0"])
		assert.equal(
			existsSync(join(fixture.output, "licenses/dependencies", name)),
			false,
		);
});
