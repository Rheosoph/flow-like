import { existsSync, readFileSync, readdirSync } from "node:fs";
import { mkdir, rm } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { parseArgs } from "node:util";
import sharp from "sharp";
import type { CaseRecord, RunManifest } from "./run";

const USAGE = `Pixel-diffs two editor-visual runs and reports what changed.

  bun scripts/editor-visual/compare.ts --baseline <run dir> --candidate <run dir> [options]

  --out <dir>             report directory (default: <candidate>/compare-<baseline label>)
  --threshold <0..1>      per-pixel YIQ color distance that counts as different (default 0.1, as pixelmatch)
  --fail-above <percent>  exit 1 when any screenshot differs by more than this, or is missing/new
`;

const PANEL_GAP = 8;
const CONCURRENCY = 8;
const MAX_YIQ_DELTA = 35215;

type Raster = {
	readonly data: Buffer;
	readonly width: number;
	readonly height: number;
};

type ErrorDelta = { readonly added: string[]; readonly removed: string[] };

type Entry = {
	readonly key: string;
	readonly status: "same" | "changed" | "resized" | "missing" | "new";
	readonly diffPixels: number;
	readonly diffPercent: number;
	readonly baselineSize: string | null;
	readonly candidateSize: string | null;
	readonly diffImage: string | null;
	readonly errors: ErrorDelta;
	readonly warnings: ErrorDelta;
	readonly externalRequests: ErrorDelta;
	readonly unsettled: boolean;
	readonly attempts: number;
};

type Run = {
	readonly dir: string;
	readonly manifest: RunManifest | null;
	readonly records: Map<string, CaseRecord>;
	readonly shots: Map<string, string>;
};

function listPngs(root: string, dir = root, found = new Map<string, string>()) {
	if (!existsSync(dir)) return found;
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) listPngs(root, path, found);
		else if (entry.name.endsWith(".png"))
			found.set(relative(root, path).replace(/\.png$/, ""), path);
	}
	return found;
}

function loadRun(dir: string): Run {
	const manifestPath = join(dir, "manifest.json");
	const manifest: RunManifest | null = existsSync(manifestPath)
		? JSON.parse(readFileSync(manifestPath, "utf8"))
		: null;
	return {
		dir,
		manifest,
		records: new Map(
			(manifest?.cases ?? []).map((record) => [record.key, record]),
		),
		shots: listPngs(join(dir, "shots")),
	};
}

async function readRaster(path: string): Promise<Raster> {
	const { data, info } = await sharp(path)
		.ensureAlpha()
		.raw()
		.toBuffer({ resolveWithObject: true });
	return { data, width: info.width, height: info.height };
}

/** pixelmatch's perceptual distance: YIQ with alpha blended over white. */
function colorDelta(a: Buffer, ai: number, b: Buffer, bi: number) {
	const blend = (value: number, alpha: number) => 255 + (value - 255) * alpha;
	const aa = a[ai + 3] / 255;
	const ba = b[bi + 3] / 255;
	const r1 = blend(a[ai], aa);
	const g1 = blend(a[ai + 1], aa);
	const b1 = blend(a[ai + 2], aa);
	const r2 = blend(b[bi], ba);
	const g2 = blend(b[bi + 1], ba);
	const b2 = blend(b[bi + 2], ba);
	const y =
		(r1 - r2) * 0.29889531 + (g1 - g2) * 0.58662247 + (b1 - b2) * 0.11448223;
	const i =
		(r1 - r2) * 0.59597799 - (g1 - g2) * 0.2741761 - (b1 - b2) * 0.32180189;
	const q =
		(r1 - r2) * 0.21147017 - (g1 - g2) * 0.52261711 + (b1 - b2) * 0.31114694;
	return 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
}

function diffRasters(baseline: Raster, candidate: Raster, threshold: number) {
	const width = Math.max(baseline.width, candidate.width);
	const height = Math.max(baseline.height, candidate.height);
	const limit = MAX_YIQ_DELTA * threshold * threshold;
	const mask = new Uint8Array(width * height);
	let diffPixels = 0;
	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const inBaseline = x < baseline.width && y < baseline.height;
			const inCandidate = x < candidate.width && y < candidate.height;
			let differs = inBaseline !== inCandidate;
			if (inBaseline && inCandidate) {
				const bi = (y * baseline.width + x) * 4;
				const ci = (y * candidate.width + x) * 4;
				const identical =
					baseline.data[bi] === candidate.data[ci] &&
					baseline.data[bi + 1] === candidate.data[ci + 1] &&
					baseline.data[bi + 2] === candidate.data[ci + 2] &&
					baseline.data[bi + 3] === candidate.data[ci + 3];
				differs =
					!identical &&
					colorDelta(baseline.data, bi, candidate.data, ci) > limit;
			}
			if (differs) {
				mask[y * width + x] = 1;
				diffPixels++;
			}
		}
	}
	return { width, height, mask, diffPixels };
}

/** baseline | candidate | differing pixels in red over a faded baseline. */
async function writeDiffImage(
	path: string,
	baseline: Raster,
	candidate: Raster,
	diff: ReturnType<typeof diffRasters>,
) {
	const { width, height, mask } = diff;
	const outWidth = width * 3 + PANEL_GAP * 2;
	const out = Buffer.alloc(outWidth * height * 4, 0);
	for (let index = 0; index < outWidth * height; index++) {
		out.writeUInt32BE(0x808080ff, index * 4);
	}
	const paint = (raster: Raster, offset: number) => {
		for (let y = 0; y < raster.height; y++) {
			raster.data.copy(
				out,
				(y * outWidth + offset) * 4,
				y * raster.width * 4,
				(y + 1) * raster.width * 4,
			);
		}
	};
	paint(baseline, 0);
	paint(candidate, width + PANEL_GAP);
	const diffOffset = (width + PANEL_GAP) * 2;
	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const target = (y * outWidth + diffOffset + x) * 4;
			if (mask[y * width + x]) {
				out.writeUInt32BE(0xff0000ff, target);
				continue;
			}
			const inBaseline = x < baseline.width && y < baseline.height;
			const source = (y * baseline.width + x) * 4;
			const luma = inBaseline
				? 0.299 * baseline.data[source] +
					0.587 * baseline.data[source + 1] +
					0.114 * baseline.data[source + 2]
				: 255;
			const faded = Math.round(255 - (255 - luma) * 0.15);
			out[target] = faded;
			out[target + 1] = faded;
			out[target + 2] = faded;
			out[target + 3] = 255;
		}
	}
	await mkdir(dirname(path), { recursive: true });
	await sharp(out, { raw: { width: outWidth, height, channels: 4 } })
		.png()
		.toFile(path);
}

const normalizeMessage = (message: string) =>
	message.replace(/\s+/g, " ").trim();

function delta(
	before: readonly string[] | undefined,
	after: readonly string[] | undefined,
): ErrorDelta {
	const left = new Set((before ?? []).map(normalizeMessage));
	const right = new Set((after ?? []).map(normalizeMessage));
	return {
		added: [...right].filter((message) => !left.has(message)),
		removed: [...left].filter((message) => !right.has(message)),
	};
}

const errorsOf = (record: CaseRecord | undefined) =>
	record
		? [
				...record.pageErrors.map((message) => `pageerror: ${message}`),
				...record.consoleErrors.map((message) => `console.error: ${message}`),
				...(record.renderError ? [`render error: ${record.renderError}`] : []),
				...(record.harnessError
					? [`harness error: ${record.harnessError}`]
					: []),
			]
		: [];

async function compareKey(
	key: string,
	baseline: Run,
	candidate: Run,
	threshold: number,
	outDir: string,
): Promise<Entry> {
	const baselineRecord = baseline.records.get(key);
	const candidateRecord = candidate.records.get(key);
	const common = {
		key,
		errors: delta(errorsOf(baselineRecord), errorsOf(candidateRecord)),
		warnings: delta(
			baselineRecord?.consoleWarnings,
			candidateRecord?.consoleWarnings,
		),
		externalRequests: delta(
			baselineRecord?.blockedRequests,
			candidateRecord?.blockedRequests,
		),
		unsettled: candidateRecord ? !candidateRecord.settled : false,
		attempts: candidateRecord?.attempts ?? 1,
	};
	const baselineShot = baseline.shots.get(key);
	const candidateShot = candidate.shots.get(key);
	if (!baselineShot || !candidateShot) {
		const shot = baselineShot ?? candidateShot;
		const raster = shot ? await sharp(shot).metadata() : null;
		const size = raster ? `${raster.width}x${raster.height}` : null;
		return {
			...common,
			status: baselineShot ? "missing" : "new",
			diffPixels: 0,
			diffPercent: 100,
			baselineSize: baselineShot ? size : null,
			candidateSize: candidateShot ? size : null,
			diffImage: null,
		};
	}
	const [before, after] = await Promise.all([
		readRaster(baselineShot),
		readRaster(candidateShot),
	]);
	const diff = diffRasters(before, after, threshold);
	const resized =
		before.width !== after.width || before.height !== after.height;
	const diffImage =
		diff.diffPixels > 0 ? join(outDir, "diff", `${key}.png`) : null;
	if (diffImage) await writeDiffImage(diffImage, before, after, diff);
	return {
		...common,
		status: resized ? "resized" : diff.diffPixels > 0 ? "changed" : "same",
		diffPixels: diff.diffPixels,
		diffPercent: (diff.diffPixels / (diff.width * diff.height)) * 100,
		baselineSize: `${before.width}x${before.height}`,
		candidateSize: `${after.width}x${after.height}`,
		diffImage: diffImage ? relative(outDir, diffImage) : null,
	};
}

const percent = (value: number) =>
	value === 0 ? "0" : value < 0.001 ? "<0.001" : value.toFixed(3);

const cell = (text: string) => text.replace(/\|/g, "\\|").replace(/\n/g, " ");

function markdownReport(
	baseline: Run,
	candidate: Run,
	entries: readonly Entry[],
	threshold: number,
) {
	const side = (run: Run, pick: (manifest: RunManifest) => string) =>
		cell(run.manifest ? pick(run.manifest) : "?");
	const row = (label: string, pick: (manifest: RunManifest) => string) =>
		`| ${label} | ${side(baseline, pick)} | ${side(candidate, pick)} |`;
	const changed = entries.filter(
		(entry) => entry.status === "changed" || entry.status === "resized",
	);
	const missing = entries.filter((entry) => entry.status === "missing");
	const added = entries.filter((entry) => entry.status === "new");
	const errorChanges = entries.filter(
		(entry) => entry.errors.added.length + entry.errors.removed.length > 0,
	);
	const requestChanges = entries.filter(
		(entry) =>
			entry.externalRequests.added.length +
				entry.externalRequests.removed.length >
			0,
	);
	const unsettled = entries.filter((entry) => entry.unsettled);
	const retried = entries.filter((entry) => entry.attempts > 1);
	const maxDiff = Math.max(
		0,
		...entries
			.filter((entry) => entry.status !== "missing" && entry.status !== "new")
			.map((entry) => entry.diffPercent),
	);
	const lines = [
		`# Editor visual diff: ${baseline.manifest?.label ?? baseline.dir} -> ${candidate.manifest?.label ?? candidate.dir}`,
		"",
		"| | baseline | candidate |",
		"| --- | --- | --- |",
		row("platejs", (manifest) => manifest.versions.platejs ?? "?"),
		row(
			"@platejs/core",
			(manifest) => manifest.versions["@platejs/core"] ?? "?",
		),
		row("react", (manifest) => manifest.versions.react ?? "?"),
		row("browser", (manifest) => manifest.browser.version),
		row("git HEAD", (manifest) => (manifest.gitHead ?? "?").slice(0, 12)),
		row("mode", (manifest) => manifest.mode),
		row("screenshots", (manifest) => String(manifest.cases.length)),
		"",
		`Per-pixel threshold ${threshold}. ${entries.length} keys, ${changed.length} changed, ${missing.length} missing, ${added.length} new. Max diff ${percent(maxDiff)}%.`,
	];
	const baselineBrowser = baseline.manifest?.browser.version;
	const candidateBrowser = candidate.manifest?.browser.version;
	if (
		baselineBrowser &&
		candidateBrowser &&
		baselineBrowser !== candidateBrowser
	) {
		lines.push(
			"",
			"**The browser versions differ, so text rasterization changes are expected everywhere.**",
		);
	}
	lines.push("", "## Changed screenshots (by diff %)", "");
	if (changed.length === 0) lines.push("None.");
	else {
		lines.push("| Screenshot | Diff % | Pixels | Size | Diff image |");
		lines.push("| --- | --- | --- | --- | --- |");
		for (const entry of changed) {
			const size =
				entry.baselineSize === entry.candidateSize
					? (entry.candidateSize ?? "")
					: `${entry.baselineSize} -> ${entry.candidateSize}`;
			lines.push(
				`| ${entry.key} | ${percent(entry.diffPercent)} | ${entry.diffPixels} | ${size} | ${entry.diffImage ? `[diff](${entry.diffImage})` : ""} |`,
			);
		}
	}
	const list = (title: string, items: readonly Entry[]) => {
		lines.push("", `## ${title}`, "");
		if (items.length === 0) lines.push("None.");
		for (const entry of items) lines.push(`- ${entry.key}`);
	};
	list("Missing in candidate", missing);
	list("New in candidate", added);
	list("Did not settle in candidate", unsettled);
	list(
		"Needed a retry in candidate (flaky, even if the pixels match)",
		retried,
	);
	lines.push("", "## Errors that changed", "");
	if (errorChanges.length === 0) lines.push("None.");
	for (const entry of errorChanges) {
		lines.push(`### ${entry.key}`);
		for (const message of entry.errors.added) lines.push(`- added: ${message}`);
		for (const message of entry.errors.removed)
			lines.push(`- removed: ${message}`);
	}
	lines.push("", "## External requests that changed (all are blocked)", "");
	if (requestChanges.length === 0) lines.push("None.");
	for (const entry of requestChanges) {
		lines.push(`### ${entry.key}`);
		for (const url of entry.externalRequests.added)
			lines.push(`- added: ${url}`);
		for (const url of entry.externalRequests.removed)
			lines.push(`- removed: ${url}`);
	}
	return { markdown: `${lines.join("\n")}\n`, maxDiff };
}

async function main() {
	const { values } = parseArgs({
		options: {
			baseline: { type: "string" },
			candidate: { type: "string" },
			out: { type: "string" },
			threshold: { type: "string", default: "0.1" },
			"fail-above": { type: "string" },
			help: { type: "boolean", default: false },
		},
	});
	if (values.help || !values.baseline || !values.candidate) {
		console.log(USAGE);
		process.exit(values.help ? 0 : 1);
	}
	const threshold = Number(values.threshold);
	if (!(threshold >= 0 && threshold <= 1))
		throw new Error(
			`editor-visual: --threshold must be 0..1, got ${values.threshold}`,
		);
	const baseline = loadRun(resolve(values.baseline));
	const candidate = loadRun(resolve(values.candidate));
	if (baseline.shots.size === 0)
		throw new Error(
			`editor-visual: no screenshots under ${baseline.dir}/shots`,
		);
	const outDir = resolve(
		values.out ??
			join(candidate.dir, `compare-${baseline.manifest?.label ?? "baseline"}`),
	);
	await rm(outDir, { recursive: true, force: true });
	await mkdir(outDir, { recursive: true });

	const keys = [
		...new Set([
			...baseline.shots.keys(),
			...candidate.shots.keys(),
			...baseline.records.keys(),
			...candidate.records.keys(),
		]),
	].sort();
	const entries: Entry[] = [];
	for (let start = 0; start < keys.length; start += CONCURRENCY) {
		const batch = keys.slice(start, start + CONCURRENCY);
		entries.push(
			...(await Promise.all(
				batch.map((key) =>
					compareKey(key, baseline, candidate, threshold, outDir),
				),
			)),
		);
	}
	entries.sort(
		(left, right) =>
			right.diffPercent - left.diffPercent || left.key.localeCompare(right.key),
	);

	const { markdown, maxDiff } = markdownReport(
		baseline,
		candidate,
		entries,
		threshold,
	);
	const summary = {
		baseline: { dir: baseline.dir, label: baseline.manifest?.label ?? null },
		candidate: { dir: candidate.dir, label: candidate.manifest?.label ?? null },
		threshold,
		maxDiffPercent: maxDiff,
		counts: Object.fromEntries(
			["same", "changed", "resized", "missing", "new"].map((status) => [
				status,
				entries.filter((entry) => entry.status === status).length,
			]),
		),
		entries,
	};
	await Bun.write(
		join(outDir, "summary.json"),
		`${JSON.stringify(summary, null, "\t")}\n`,
	);
	await Bun.write(join(outDir, "summary.md"), markdown);

	console.log(
		`${entries.length} keys: ${Object.entries(summary.counts)
			.map(([status, count]) => `${count} ${status}`)
			.join(", ")}; max diff ${percent(maxDiff)}%`,
	);
	for (const entry of entries
		.filter((item) => item.status !== "same")
		.slice(0, 20)) {
		console.log(
			`  ${entry.status.padEnd(8)} ${percent(entry.diffPercent).padStart(8)}%  ${entry.key}`,
		);
	}
	console.log(`report: ${join(outDir, "summary.md")}`);

	const failAbove = values["fail-above"];
	if (failAbove !== undefined) {
		const limit = Number(failAbove);
		const failing = entries.filter(
			(entry) =>
				entry.status === "missing" ||
				entry.status === "new" ||
				entry.diffPercent > limit,
		);
		if (failing.length > 0) process.exitCode = 1;
	}
}

await main();
