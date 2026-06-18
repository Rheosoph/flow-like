import {
	existsSync,
	mkdirSync,
	readFileSync,
	readdirSync,
	statSync,
	writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = path.resolve(SCRIPT_DIR, "..");
const ICON_REFERENCE_PATTERN = /\/flow\/icons\/([a-zA-Z0-9_.-]+\.svg)/g;

export const NODE_ICON_PUBLIC_DIRS = [
	"apps/docs/public/flow/icons",
	"apps/desktop/public/flow/icons",
	"apps/web/public/flow/icons",
	"apps/embedded/public/flow/icons",
] as const;

const LUCIDE_ICON_MAP: Record<string, string> = {
	"3d.svg": "axis-3d",
	"audio.svg": "audio-lines",
	"automation.svg": "workflow",
	"aws.svg": "cloud",
	"bell-ring.svg": "bell-ring",
	"bell.svg": "bell",
	"book-key.svg": "book-key",
	"bot-search.svg": "bot-message-square",
	"box.svg": "box",
	"chart.svg": "chart-column",
	"cloud.svg": "cloud",
	"compare.svg": "git-compare-arrows",
	"compress.svg": "minimize-2",
	"crop.svg": "crop",
	"depth.svg": "layers-3",
	"discord.svg": "messages-square",
	"face.svg": "scan-face",
	"file-text.svg": "file-text",
	"file.svg": "file",
	"files.svg": "files",
	"filter.svg": "filter",
	"gate.svg": "git-branch",
	"hexagon.svg": "hexagon",
	"info.svg": "info",
	"key.svg": "key",
	"layers.svg": "layers",
	"linkedin.svg": "briefcase-business",
	"lock.svg": "lock",
	"log-progress-done.svg": "circle-check",
	"log-progress.svg": "loader",
	"map-pin.svg": "map-pin",
	"map.svg": "map",
	"microphone.svg": "mic",
	"palette.svg": "palette",
	"play.svg": "play",
	"route.svg": "route",
	"scissors.svg": "scissors",
	"settings.svg": "settings",
	"shape.svg": "shapes",
	"shield-ai.svg": "shield-alert",
	"shield.svg": "shield",
	"smartphone.svg": "smartphone",
	"sparkles.svg": "sparkles",
	"table.svg": "table",
	"tag.svg": "tag",
	"text-search.svg": "scan-text",
	"text.svg": "text",
	"type.svg": "type",
	"unlock.svg": "unlock",
	"user.svg": "user",
	"video.svg": "video",
	"waveform.svg": "audio-waveform",
};

export type NodeIconIssueType =
	| "missing-source"
	| "missing-target"
	| "stale-target";

export interface NodeIconIssue {
	type: NodeIconIssueType;
	icon: string;
	target?: string;
	message: string;
}

export interface NodeIconCheckResult {
	icons: string[];
	issues: NodeIconIssue[];
	updated: string[];
}

interface SyncOptions {
	check?: boolean;
	root?: string;
}

function walkFiles(dir: string, extension: string): string[] {
	if (!existsSync(dir)) return [];

	const files: string[] = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (
			entry.name === "target" ||
			entry.name === "node_modules" ||
			entry.name === ".next"
		) {
			continue;
		}

		const fullPath = path.join(dir, entry.name);
		if (entry.isDirectory()) {
			files.push(...walkFiles(fullPath, extension));
			continue;
		}

		if (entry.isFile() && fullPath.endsWith(extension)) {
			files.push(fullPath);
		}
	}

	return files;
}

export function collectReferencedNodeIcons(root = DEFAULT_ROOT): string[] {
	const catalogDir = path.join(root, "packages", "catalog");
	const icons = new Set<string>();

	for (const file of walkFiles(catalogDir, ".rs")) {
		const source = readFileSync(file, "utf8");
		for (const match of source.matchAll(ICON_REFERENCE_PATTERN)) {
			icons.add(match[1]);
		}
	}

	return [...icons].sort();
}

function findExistingIcon(root: string, icon: string): string | undefined {
	for (const dir of NODE_ICON_PUBLIC_DIRS) {
		const iconPath = path.join(root, dir, icon);
		if (existsSync(iconPath) && statSync(iconPath).isFile()) {
			return iconPath;
		}
	}

	return undefined;
}

function readIconSource(root: string, icon: string): string | undefined {
	const sourcePath = findExistingIcon(root, icon);
	return sourcePath ? readFileSync(sourcePath, "utf8") : undefined;
}

function escapeAttribute(value: string | number | boolean): string {
	return String(value)
		.replaceAll("&", "&amp;")
		.replaceAll('"', "&quot;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;");
}

function renderSvgElement(
	tag: string,
	attributes: Record<string, string | number | boolean>,
): string {
	const attrs = Object.entries(attributes)
		.filter(([key]) => key !== "key")
		.map(([key, value]) => `${key}="${escapeAttribute(value)}"`)
		.join(" ");

	return attrs ? `<${tag} ${attrs}/>` : `<${tag}/>`;
}

function resolveLucideModulePath(root: string, lucideName: string): string {
	let currentName = lucideName;
	const seen = new Set<string>();

	for (;;) {
		if (seen.has(currentName)) {
			throw new Error(`Lucide icon ${lucideName} has a circular re-export`);
		}
		seen.add(currentName);

		const modulePath = path.join(
			root,
			"node_modules",
			"lucide-react",
			"dist",
			"esm",
			"icons",
			`${currentName}.js`,
		);

		if (!existsSync(modulePath)) {
			throw new Error(`Lucide icon ${currentName} was not found`);
		}

		const source = readFileSync(modulePath, "utf8");
		const reexport = source.match(
			/export \{ default \} from '\.\/([^']+)\.js';/,
		);
		if (!reexport) return modulePath;

		currentName = reexport[1];
	}
}

async function renderLucideIcon(root: string, icon: string): Promise<string> {
	const lucideName = LUCIDE_ICON_MAP[icon];
	if (!lucideName) {
		throw new Error(`No Lucide fallback is configured for ${icon}`);
	}

	const modulePath = resolveLucideModulePath(root, lucideName);

	const module = (await import(pathToFileURL(modulePath).href)) as {
		__iconNode?: Array<[string, Record<string, string | number | boolean>]>;
	};

	if (!module.__iconNode) {
		throw new Error(`Lucide icon ${lucideName} does not export __iconNode`);
	}

	const children = module.__iconNode
		.map(([tag, attributes]) => renderSvgElement(tag, attributes))
		.join("");

	return `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-${lucideName}">${children}</svg>\n`;
}

function compareTargets(
	root: string,
	icon: string,
	source: string | undefined,
): NodeIconIssue[] {
	const issues: NodeIconIssue[] = [];

	if (!source) {
		issues.push({
			type: "missing-source",
			icon,
			message: `No public SVG exists for /flow/icons/${icon}. Run bun scripts/sync-node-icons.ts to create it from Lucide or add a source SVG manually.`,
		});
		return issues;
	}

	for (const dir of NODE_ICON_PUBLIC_DIRS) {
		const target = path.join(root, dir, icon);
		const publicTarget = path.join(dir, icon);

		if (!existsSync(target)) {
			issues.push({
				type: "missing-target",
				icon,
				target: publicTarget,
				message: `${publicTarget} is missing.`,
			});
			continue;
		}

		if (readFileSync(target, "utf8") !== source) {
			issues.push({
				type: "stale-target",
				icon,
				target: publicTarget,
				message: `${publicTarget} differs from the synced source icon.`,
			});
		}
	}

	return issues;
}

async function writeTargets(
	root: string,
	icon: string,
	source: string,
): Promise<string[]> {
	const updated: string[] = [];

	for (const dir of NODE_ICON_PUBLIC_DIRS) {
		const targetDir = path.join(root, dir);
		const target = path.join(targetDir, icon);
		mkdirSync(targetDir, { recursive: true });

		if (existsSync(target) && readFileSync(target, "utf8") === source) {
			continue;
		}

		writeFileSync(target, source);
		updated.push(path.join(dir, icon));
	}

	return updated;
}

export async function syncNodeIcons(
	options: SyncOptions = {},
): Promise<NodeIconCheckResult> {
	const root = path.resolve(options.root ?? DEFAULT_ROOT);
	const icons = collectReferencedNodeIcons(root);
	const issues: NodeIconIssue[] = [];
	const updated: string[] = [];

	for (const icon of icons) {
		let source = readIconSource(root, icon);

		if (!source && !options.check) {
			source = await renderLucideIcon(root, icon);
		}

		const iconIssues = compareTargets(root, icon, source);
		if (options.check) {
			issues.push(...iconIssues);
			continue;
		}

		if (!source) {
			issues.push(...iconIssues);
			continue;
		}

		updated.push(...(await writeTargets(root, icon, source)));
	}

	if (!options.check) {
		const postSyncIssues = icons.flatMap((icon) =>
			compareTargets(root, icon, readIconSource(root, icon)),
		);
		issues.splice(0, issues.length, ...postSyncIssues);
	}

	return { icons, issues, updated };
}

export function checkNodeIcons(root = DEFAULT_ROOT): NodeIconCheckResult {
	const icons = collectReferencedNodeIcons(root);
	const issues = icons.flatMap((icon) =>
		compareTargets(root, icon, readIconSource(root, icon)),
	);

	return { icons, issues, updated: [] };
}

export function formatNodeIconIssues(issues: NodeIconIssue[]): string {
	if (issues.length === 0) return "All node icons are present and synced.";

	return issues.map((issue) => issue.message).join("\n");
}

async function main() {
	const check = process.argv.includes("--check");
	const result = check ? checkNodeIcons() : await syncNodeIcons();

	if (result.issues.length > 0) {
		console.error(formatNodeIconIssues(result.issues));
		process.exitCode = 1;
		return;
	}

	if (check) {
		console.log(`Checked ${result.icons.length} node icons.`);
		return;
	}

	console.log(
		result.updated.length === 0
			? `Node icons are already synced (${result.icons.length} icons).`
			: `Synced ${result.updated.length} public node icon files (${result.icons.length} referenced icons).`,
	);
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
	await main();
}
