import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { acquireBuildLock } from "./build-lock.mjs";

const editors = new Set(["wordcraft", "pdfcraft", "photocraft", "vectorcraft"]);
const repository = "https://github.com/dtolnay/inventory";
const revision = "ffc228473d7aa20ca0790ddccc894a20682489ae";
const checksum =
	"6928282826c822ad91bf1c9a1cb90a30ba1c26770749929b4656cd6be829cd7c";
const packagePath = "host-dependencies/inventory";
const dependency = `inventory = { path = "${packagePath}" }`;
const selector = "any(all(stable, since(1.85)), since(2024-12-18))";
const betaSelector =
	"any(all(any(stable, beta), since(1.85)), since(2024-12-18))";

export function assertInventoryLock(lock) {
	const entries = lock
		.split(/^\[\[package\]\]\s*$/m)
		.filter((block) => /^name = "inventory"$/m.test(block));
	const field = (block, name) => {
		const matches = [
			...block.matchAll(new RegExp(`^${name} = ("[^"\\n]*")$`, "gm")),
		];
		return matches.length === 1 ? JSON.parse(matches[0][1]) : undefined;
	};
	if (
		entries.length !== 1 ||
		field(entries[0], "version") !== "0.3.25" ||
		field(entries[0], "source") !==
			"registry+https://github.com/rust-lang/crates.io-index" ||
		field(entries[0], "checksum") !== checksum
	)
		throw new Error(
			"Review the pinned inventory registry package before patching it.",
		);
}

export function patchInventoryLibrary(text, replaceOnce) {
	return replaceOnce(
		text,
		selector,
		betaSelector,
		"inventory beta constructor section",
	);
}

export function patchInventoryManifest(text) {
	for (const [name, value] of [
		["name", "inventory"],
		["version", "0.3.25"],
		["repository", repository],
		["license", "MIT OR Apache-2.0"],
	]) {
		if (!text.includes(`${name} = ${JSON.stringify(value)}\n`))
			throw new Error(`The pinned inventory ${name} changed.`);
	}
	const dev = /^\[dev-dependencies\]\r?\n[\s\S]*?(?=^\[|(?![\s\S]))/gm;
	if ([...text.matchAll(dev)].length !== 1)
		throw new Error("The pinned inventory development dependencies changed.");
	return text.replace(dev, "");
}

export function addInventoryPatch(workspace) {
	const heading =
		/^\[patch\.(?:crates-io|"crates-io"|'crates-io')\][ \t]*(?:#.*)?$/gm;
	const headings = [...workspace.matchAll(heading)];
	if (
		headings.length > 1 ||
		/^\[patch\.(?:crates-io|"crates-io"|'crates-io')\.(?:inventory|"inventory"|'inventory')\]/m.test(
			workspace,
		)
	)
		throw new Error("Review the existing inventory crates.io patch.");
	if (!headings.length)
		return `${workspace.trimEnd()}\n\n[patch.crates-io]\n${dependency}\n`;
	const match = headings[0];
	const start = match.index + match[0].length;
	const next = workspace.slice(start).search(/^\[/m);
	const end = next < 0 ? workspace.length : start + next;
	const section = workspace.slice(start, end);
	const entries = [
		...section.matchAll(
			/^[ \t]*(?:inventory|"inventory"|'inventory')[ \t]*=([^\r\n]*)$/gm,
		),
	];
	if (entries.length) {
		if (entries.length !== 1 || entries[0][0].trim() !== dependency)
			throw new Error(
				"Refusing to replace an existing inventory crates.io patch.",
			);
		return workspace;
	}
	return `${workspace.slice(0, start)}\n${dependency}${workspace.slice(start)}`;
}

export function prepareInventory({
	id,
	root,
	source,
	original,
	write,
	replaceOnce,
}) {
	if (!editors.has(id)) return [];
	assertInventoryLock(original("Cargo.lock"));
	const workspace = addInventoryPatch(
		readFileSync(join(source, "Cargo.toml"), "utf8"),
	);
	const cache = join(root, "target/craft-viewers/dependencies/inventory");
	const release = acquireBuildLock({
		directory: cache,
		label: "inventory dependency",
	});
	const checkout = join(cache, revision);
	const git = (args, encoding = "utf8") =>
		execFileSync("git", args, {
			cwd: checkout,
			encoding,
			maxBuffer: 8 * 1024 * 1024,
			stdio: ["ignore", "pipe", "pipe"],
		});
	try {
		mkdirSync(checkout, { recursive: true });
		if (!existsSync(join(checkout, ".git"))) git(["init", "--quiet"]);
		try {
			git(["cat-file", "-e", `${revision}^{commit}`]);
		} catch {
			git(["fetch", "--depth", "1", repository, revision]);
		}
		const read = (path) => git(["show", `${revision}:${path}`]);
		const manifest = patchInventoryManifest(read("Cargo.toml"));
		const lib = patchInventoryLibrary(read("src/lib.rs"), replaceOnce);
		const files = git(["ls-tree", "-rz", revision, "--", "src"])
			.split("\0")
			.filter(Boolean);
		if (!files.length) throw new Error("The pinned inventory source is empty.");
		for (const entry of files) {
			const match = entry.match(/^100644 blob [a-f0-9]{40}\t(src\/[^\0]+)$/);
			if (!match || match[1].split("/").includes(".."))
				throw new Error("The pinned inventory source tree changed.");
			const path = match[1];
			write(
				`${packagePath}/${path}`,
				path === "src/lib.rs"
					? lib
					: git(["show", `${revision}:${path}`], null),
			);
		}
		for (const license of ["LICENSE-MIT", "LICENSE-APACHE"])
			write(`${packagePath}/${license}`, read(license));
		write(`${packagePath}/Cargo.toml`, manifest);
		write("Cargo.toml", workspace);
	} finally {
		release();
	}
	return [
		{
			name: "inventory",
			version: "0.3.25",
			source: `git+${repository}?rev=${revision}#${revision}`,
			path: packagePath,
		},
	];
}
