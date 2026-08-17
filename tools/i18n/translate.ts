/**
 * Safely fill empty locale values with machine translations.
 *
 * Dry-run is the default. Pass `--apply` to write successful translations.
 * Existing non-empty translations are never changed. Strings classified as
 * code by the instrumentation filter are copied byte-for-byte.
 */
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { isProse } from "./instrument";

const REPO_ROOT = path.resolve(import.meta.dirname, "../..");
const LOCALES_DIR = path.join(REPO_ROOT, "packages/locales/locales");
const ENDPOINT = "https://translate.googleapis.com/translate_a/single";
const SEPARATOR = "[[[FLOWLIKETRANSLATIONSEPARATOR3F0A9D]]]";
const TONE_PREFIX = "Hey friend, ";
const MAX_BATCH_ITEMS = 20;
const MAX_BATCH_CHARACTERS = 3_500;

interface Args {
	source: string;
	target: string;
	namespaces: string[];
	apply: boolean;
	copyTechnicalOnly: boolean;
	allowExternalProvider: boolean;
	localModel?: string;
	python: string;
}

interface Entry {
	key: string;
	source: string;
}

interface ProtectedValue {
	text: string;
	tokens: string[];
}

interface Failure extends Entry {
	reason: string;
}

const TAILWIND_BARE_UTILITIES = new Set([
	"absolute",
	"block",
	"contents",
	"container",
	"flex",
	"fixed",
	"grid",
	"group",
	"hidden",
	"inline",
	"isolate",
	"relative",
	"shrink",
	"static",
	"sticky",
	"table",
	"truncate",
	"visible",
]);

function looksLikeTailwind(value: string): boolean {
	const tokens = value.trim().split(/\s+/);
	if (tokens.length < 2) return false;
	const matches = tokens.filter((token) => {
		const normalized = token.replace(/^!/, "");
		return (
			TAILWIND_BARE_UTILITIES.has(normalized) ||
			/^(?:[a-z-]+:)*-?(?:m[trblxy]?|p[trblxy]?|h|min-h|max-h|w|min-w|max-w|gap|space-[xy]|inset|top|right|bottom|left|z|order|col|row|grid|flex|items|justify|content|self|place|overflow|overscroll|text|font|leading|tracking|bg|from|via|to|border|rounded|ring|shadow|opacity|transition|duration|ease|animate|cursor|select|resize|object|aspect|origin|translate|rotate|scale|skew|fill|stroke|outline|decoration|whitespace|break|align|basis|grow|shrink)-[^\s]+$/.test(
				normalized,
			) ||
			/^\[&[^\]]*\]:/.test(normalized)
		);
	});
	return matches.length >= 2 && matches.length / tokens.length >= 0.5;
}

function looksLikeSourceCode(value: string): boolean {
	const trimmed = value.trim();
	const withoutInterpolations = trimmed.replace(/\{\{[^}]+\}\}/g, "");
	return (
		/^(?:#?[0-9a-f]{3,8}|(?:bg|text|border)-[\w-[\]()]+)\s+or\s+(?:#?[0-9a-f]{3,8}|(?:bg|text|border)-[\w-[\]()]+)$/i.test(
			trimmed,
		) ||
		/^(?:minmax|repeat|fit-content)\(/i.test(trimmed) ||
		/^\{\{[^}]+\}\}\s*;\s*(?:font|background|color|border|padding|margin|display|position)-?[\w-]*\s*:/i.test(
			trimmed,
		) ||
		/^(?:from\s+[\w.]+\s+import|import\s+[\w{*]|export\s+(?:default|const|function|class)|const\s+|let\s+|var\s+|function\s+|class\s+)/.test(
			trimmed,
		) ||
		/^(?:curl|wget|brew|winget|choco|npm|bun|pnpm|yarn|git|sudo|docker|kubectl|npx|mise)\s/i.test(
			trimmed,
		) ||
		/^<!(?:doctype)|^<(?:html|head|body|style|script|link)\b/i.test(trimmed) ||
		/^\s*(?:input|select|textarea|button)(?:\s*,\s*(?:input|select|textarea|button))+\s*$/i.test(
			trimmed,
		) ||
		(/[{};]/.test(withoutInterpolations) &&
			/\b(?:await|return|new|client|response|println|print|className|style|onClick|onChange)\b/.test(
				withoutInterpolations,
			))
	);
}

export function shouldTranslate(value: string): boolean {
	return (
		isProse(value) && !looksLikeTailwind(value) && !looksLikeSourceCode(value)
	);
}

function parseArgs(argv: string[]): Args {
	const args: Args = {
		source: "en",
		target: "de",
		namespaces: [],
		apply: false,
		copyTechnicalOnly: false,
		allowExternalProvider: false,
		localModel: undefined,
		python: "python3",
	};
	for (let index = 0; index < argv.length; index++) {
		const arg = argv[index];
		if (arg === "--source") args.source = argv[++index];
		else if (arg === "--target") args.target = argv[++index];
		else if (arg === "--namespace") args.namespaces.push(argv[++index]);
		else if (arg === "--apply") args.apply = true;
		else if (arg === "--copy-technical-only") args.copyTechnicalOnly = true;
		else if (arg === "--allow-external-provider")
			args.allowExternalProvider = true;
		else if (arg === "--local-model") args.localModel = argv[++index];
		else if (arg === "--python") args.python = argv[++index];
		else throw new Error(`unknown argument: ${arg}`);
	}
	return args;
}

function flatten(
	tree: Record<string, unknown>,
	prefix = "",
	out: Record<string, string> = {},
): Record<string, string> {
	for (const [key, value] of Object.entries(tree)) {
		const fullKey = prefix ? `${prefix}.${key}` : key;
		if (typeof value === "string") out[fullKey] = value;
		else if (value && typeof value === "object" && !Array.isArray(value)) {
			flatten(value as Record<string, unknown>, fullKey, out);
		}
	}
	return out;
}

function setPath(
	tree: Record<string, unknown>,
	key: string,
	value: string,
): void {
	const segments = key.split(".");
	const leaf = segments.at(-1);
	if (!leaf) throw new Error("translation key must not be empty");
	let cursor = tree;
	for (let index = 0; index < segments.length - 1; index++) {
		cursor = cursor[segments[index]] as Record<string, unknown>;
	}
	cursor[leaf] = value;
}

const PROTECTED_PATTERN =
	/\{\{\s*[^}]+?\s*\}\}|\$t\([^)]+\)|<\/?\d+\s*\/?>|<\/?[A-Za-z][^>]*>|https?:\/\/[^\s<>]+|\b[^\s@]+@[^\s@]+\.[^\s@]+|```[\s\S]*?```|`[^`]+`|\b(?:Flow-Like|FlowLike|FlowPilot|Flows?|Boards?|Nodes?|Apps?|WASM|WebAssembly|WebSockets?|APIs?|URLs?|JSON|OAuth|MCP|LLMs?|VLMs?|GGUF|MLX|HTTP|HTTPS|REST|SQL|CSV|Markdown|GitHub|Codex|ngrok|Cloudflare|Telegram|Discord|SDK|CLI|SSE|PAT|IDs?|Hubs?)\b/g;

function protect(value: string): ProtectedValue {
	const tokens: string[] = [];
	const text = value.replace(PROTECTED_PATTERN, (token) => {
		const index = tokens.push(token) - 1;
		return `ZZFLTOKEN${index.toString().padStart(4, "0")}ZZ`;
	});
	return { text, tokens };
}

function restore(value: string, tokens: string[]): string {
	let output = value;
	for (let index = 0; index < tokens.length; index++) {
		const marker = `ZZFLTOKEN${index.toString().padStart(4, "0")}ZZ`;
		const matches = output.match(new RegExp(marker, "g")) ?? [];
		if (matches.length !== 1) {
			throw new Error(`${marker} occurred ${matches.length} times`);
		}
		output = output.replace(marker, tokens[index]);
	}
	if (/ZZFLTOKEN\d+ZZ/.test(output)) throw new Error("unknown token marker");
	return output;
}

function significantTokens(value: string): string[] {
	return [...value.matchAll(/\{\{\s*([^}]+?)\s*\}\}|\$t\(([^)]+)\)/g)]
		.map((match) => (match[1] ?? `$t(${match[2]})`).trim())
		.sort();
}

function transTags(value: string): string[] {
	return [...value.matchAll(/<\s*(\/?)\s*(\d+)\s*(\/?)\s*>/g)]
		.map((match) =>
			match[1]
				? `</${match[2]}>`
				: match[3]
					? `<${match[2]}/>`
					: `<${match[2]}>`,
		)
		.sort();
}

function sameList(left: string[], right: string[]): boolean {
	return JSON.stringify(left) === JSON.stringify(right);
}

function validate(source: string, translated: string): void {
	if (!translated.trim()) throw new Error("empty translation");
	if (translated.includes(SEPARATOR)) throw new Error("separator leaked");
	if (!sameList(significantTokens(source), significantTokens(translated))) {
		throw new Error("interpolation or nesting tokens changed");
	}
	if (!sameList(transTags(source), transTags(translated))) {
		throw new Error("Trans tags changed");
	}
	for (const match of translated.matchAll(/\{\{\s*([^}]+?)\s*\}\}/g)) {
		const token = match[0];
		const name = match[1].trim();
		const start = match.index ?? 0;
		const end = start + token.length;
		const targetBefore = translated.slice(0, start).at(-1) ?? "";
		const targetAfter = translated[end] ?? "";
		const sourceOccurrences = [
			...source.matchAll(
				new RegExp(`\\{\\{\\s*${escapeRegExp(name)}\\s*\\}\\}`, "g"),
			),
		];
		const sourceHasLetterBefore = sourceOccurrences.some((occurrence) => {
			const index = occurrence.index ?? 0;
			return /\p{L}/u.test(source.slice(0, index).at(-1) ?? "");
		});
		const sourceHasLetterAfter = sourceOccurrences.some((occurrence) => {
			const index = (occurrence.index ?? 0) + occurrence[0].length;
			return /\p{L}/u.test(source[index] ?? "");
		});
		if (/\p{L}/u.test(targetBefore) && !sourceHasLetterBefore) {
			throw new Error(`word joined before {{${name}}}`);
		}
		if (/\p{L}/u.test(targetAfter) && !sourceHasLetterAfter) {
			throw new Error(`word joined after {{${name}}}`);
		}
	}
	const sourceControls = [...source].filter(
		(character) =>
			character.charCodeAt(0) < 32 && character !== "\n" && character !== "\t",
	);
	const targetControls = [...translated].filter(
		(character) =>
			character.charCodeAt(0) < 32 && character !== "\n" && character !== "\t",
	);
	if (!sameList(sourceControls, targetControls)) {
		throw new Error("control characters changed");
	}
}

function escapeRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function translatedText(payload: unknown): string {
	if (!Array.isArray(payload) || !Array.isArray(payload[0])) {
		throw new Error("unexpected provider response");
	}
	return payload[0]
		.map((segment: unknown) =>
			Array.isArray(segment) && typeof segment[0] === "string"
				? segment[0]
				: "",
		)
		.join("");
}

function request(text: string, source: string, target: string): string {
	let error = "translation request failed";
	for (let attempt = 1; attempt <= 4; attempt++) {
		const result = spawnSync(
			"curl",
			[
				"-fsS",
				"--max-time",
				"45",
				"--retry",
				"2",
				"--retry-all-errors",
				"--get",
				"--data-urlencode",
				"client=gtx",
				"--data-urlencode",
				`sl=${source}`,
				"--data-urlencode",
				`tl=${target}`,
				"--data-urlencode",
				"dt=t",
				"--data-urlencode",
				`q=${text}`,
				ENDPOINT,
			],
			{ encoding: "utf8", maxBuffer: 2 * 1024 * 1024 },
		);
		if (result.status === 0) {
			try {
				return translatedText(JSON.parse(result.stdout));
			} catch (caught) {
				error = caught instanceof Error ? caught.message : String(caught);
			}
		} else {
			error = result.stderr.trim() || `curl exited ${result.status}`;
		}
		if (attempt < 4) Bun.sleepSync(attempt * 500);
	}
	throw new Error(error);
}

function requestLocal(texts: string[], args: Args): string[] {
	if (!args.localModel) throw new Error("local model path is required");
	const worker = path.join(REPO_ROOT, "tools/i18n/translate-local.py");
	const result = spawnSync(args.python, [worker, "--model", args.localModel], {
		encoding: "utf8",
		input: JSON.stringify(texts),
		maxBuffer: 64 * 1024 * 1024,
		stdio: ["pipe", "pipe", "inherit"],
		env: { ...process.env, TOKENIZERS_PARALLELISM: "false" },
	});
	if (result.status !== 0) {
		throw new Error(`local translation worker exited ${result.status}`);
	}
	const output = JSON.parse(result.stdout) as unknown;
	if (
		!Array.isArray(output) ||
		output.length !== texts.length ||
		!output.every((value) => typeof value === "string")
	) {
		throw new Error("local translation worker returned an invalid result set");
	}
	return output as string[];
}

const PREFIX_PATTERN =
	/^(?:Hey|Hallo),?\s+(?:mein(?:e)?\s+)?(?:Freund(?:in)?|Kumpel),?\s*/i;

function removeTonePrefix(value: string): string {
	const stripped = value.replace(PREFIX_PATTERN, "");
	if (stripped === value)
		throw new Error("informal-tone prefix was not preserved");
	return stripped.charAt(0).toLocaleUpperCase("de-DE") + stripped.slice(1);
}

function hasFormalAddress(value: string): boolean {
	return /\b(?:Sie|Ihnen|Ihr|Ihre|Ihrer|Ihrem|Ihren|Ihres)\b/.test(value);
}

function shouldRetrySourceEqual(
	source: string,
	protectedText: string,
): boolean {
	const unprotected = protectedText.replace(/ZZFLTOKEN\d+ZZ/g, "");
	return source.trim().length > 1 && /\p{L}{2,}/u.test(unprotected);
}

function translateLocally(
	entries: Entry[],
	args: Args,
): { values: Map<string, string>; failures: Failure[] } {
	const prepared = entries.map((entry) => ({
		...entry,
		protected: protect(entry.source),
	}));
	const initial = requestLocal(
		prepared.map((entry) => entry.protected.text),
		args,
	);
	const values = new Map<string, string>();
	const retry: number[] = [];
	const initialErrors = new Map<number, string>();

	for (let index = 0; index < prepared.length; index++) {
		const entry = prepared[index];
		try {
			const restored = restore(initial[index].trim(), entry.protected.tokens);
			validate(entry.source, restored);
			if (
				hasFormalAddress(restored) ||
				(restored === entry.source &&
					shouldRetrySourceEqual(entry.source, entry.protected.text))
			) {
				retry.push(index);
			} else {
				values.set(entry.key, restored);
			}
		} catch (caught) {
			initialErrors.set(
				index,
				caught instanceof Error ? caught.message : String(caught),
			);
			retry.push(index);
		}
	}

	if (retry.length) {
		console.log(
			`local model: retrying ${retry.length} formal, copied, or invalid result(s)`,
		);
		const retried = requestLocal(
			retry.map((index) => `${TONE_PREFIX}${prepared[index].protected.text}`),
			args,
		);
		for (let position = 0; position < retry.length; position++) {
			const index = retry[position];
			const entry = prepared[index];
			try {
				const withoutPrefix = removeTonePrefix(retried[position].trim());
				const restored = restore(withoutPrefix, entry.protected.tokens);
				validate(entry.source, restored);
				if (hasFormalAddress(restored)) {
					throw new Error("retry still uses formal address");
				}
				if (
					restored === entry.source &&
					shouldRetrySourceEqual(entry.source, entry.protected.text)
				) {
					throw new Error("retry copied translatable source text");
				}
				values.set(entry.key, restored);
			} catch (caught) {
				initialErrors.set(
					index,
					caught instanceof Error ? caught.message : String(caught),
				);
			}
		}
	}

	const failures = prepared
		.map((entry, index) =>
			values.has(entry.key)
				? undefined
				: {
						key: entry.key,
						source: entry.source,
						reason: initialErrors.get(index) ?? "translation was not produced",
					},
		)
		.filter((failure): failure is Failure => failure !== undefined);
	return { values, failures };
}

function translateBatch(
	entries: Entry[],
	sourceLanguage: string,
	targetLanguage: string,
): Map<string, string> {
	const prepared = entries.map((entry) => ({
		...entry,
		protected: protect(entry.source),
	}));
	const joined = prepared
		.map((entry) => `${TONE_PREFIX}${entry.protected.text}`)
		.join(`\n${SEPARATOR}\n`);
	const translated = request(joined, sourceLanguage, targetLanguage);
	const parts = translated.split(new RegExp(`\\s*${SEPARATOR}\\s*`, "g"));
	if (parts.length !== entries.length) {
		throw new Error(
			`expected ${entries.length} results, received ${parts.length}`,
		);
	}
	const output = new Map<string, string>();
	for (let index = 0; index < prepared.length; index++) {
		const entry = prepared[index];
		const withoutPrefix = removeTonePrefix(parts[index].trim());
		const restored = restore(withoutPrefix, entry.protected.tokens);
		validate(entry.source, restored);
		output.set(entry.key, restored);
	}
	return output;
}

function translateOne(
	entry: Entry,
	sourceLanguage: string,
	targetLanguage: string,
): string {
	const protectedValue = protect(entry.source);
	let raw: string;
	try {
		raw = removeTonePrefix(
			request(
				`${TONE_PREFIX}${protectedValue.text}`,
				sourceLanguage,
				targetLanguage,
			).trim(),
		);
	} catch {
		// Provider occasionally drops the tone prefix for terse labels. Translate
		// those without it; QA flags formal pronouns for manual correction.
		raw = request(protectedValue.text, sourceLanguage, targetLanguage).trim();
	}
	const restored = restore(raw, protectedValue.tokens);
	validate(entry.source, restored);
	return restored;
}

function batches(entries: Entry[]): Entry[][] {
	const output: Entry[][] = [];
	let current: Entry[] = [];
	let characters = 0;
	for (const entry of entries) {
		const size =
			entry.source.length + TONE_PREFIX.length + SEPARATOR.length + 2;
		if (
			current.length > 0 &&
			(current.length >= MAX_BATCH_ITEMS ||
				characters + size > MAX_BATCH_CHARACTERS)
		) {
			output.push(current);
			current = [];
			characters = 0;
		}
		current.push(entry);
		characters += size;
	}
	if (current.length) output.push(current);
	return output;
}

function main(): void {
	const args = parseArgs(process.argv.slice(2));
	if (
		args.apply &&
		!args.copyTechnicalOnly &&
		!args.localModel &&
		!args.allowExternalProvider
	) {
		throw new Error(
			"--apply sends source strings to translate.googleapis.com; pass --allow-external-provider only after approving that disclosure",
		);
	}
	const config = JSON.parse(
		readFileSync(path.join(LOCALES_DIR, "config.json"), "utf8"),
	) as { sourceLanguage: string; languages: string[]; namespaces: string[] };
	if (
		!config.languages.includes(args.source) ||
		!config.languages.includes(args.target)
	) {
		throw new Error("source and target must be configured locales");
	}
	const namespaces = args.namespaces.length
		? args.namespaces
		: config.namespaces;
	for (const namespace of namespaces) {
		if (!config.namespaces.includes(namespace)) {
			throw new Error(`unknown namespace: ${namespace}`);
		}
	}

	let copied = 0;
	let translated = 0;
	const failures: Failure[] = [];
	for (const namespace of namespaces) {
		const sourcePath = path.join(LOCALES_DIR, args.source, `${namespace}.json`);
		const targetPath = path.join(LOCALES_DIR, args.target, `${namespace}.json`);
		const sourceTree = JSON.parse(readFileSync(sourcePath, "utf8")) as Record<
			string,
			unknown
		>;
		const targetTree = JSON.parse(readFileSync(targetPath, "utf8")) as Record<
			string,
			unknown
		>;
		const source = flatten(sourceTree);
		const target = flatten(targetTree);
		const empty = Object.entries(source)
			.filter(([key]) => !(target[key] ?? "").trim())
			.map(([key, value]) => ({ key, source: value }));
		const prose: Entry[] = [];
		for (const entry of empty) {
			if (shouldTranslate(entry.source)) prose.push(entry);
			else {
				setPath(targetTree, entry.key, entry.source);
				copied++;
			}
		}

		const work = batches(prose);
		if (!args.apply) {
			console.log(
				`${namespace}: ${prose.length} prose translation(s), ${empty.length - prose.length} exact technical copy/copies`,
			);
			continue;
		}
		if (args.copyTechnicalOnly) {
			writeFileSync(
				targetPath,
				`${JSON.stringify(targetTree, null, "\t")}\n`,
				"utf8",
			);
			console.log(
				`${namespace}: wrote exact technical copies; left ${prose.length} prose values empty`,
			);
			continue;
		}
		if (args.localModel) {
			const local = translateLocally(prose, args);
			for (const [key, value] of local.values) setPath(targetTree, key, value);
			translated += local.values.size;
			failures.push(...local.failures);
			writeFileSync(
				targetPath,
				`${JSON.stringify(targetTree, null, "\t")}\n`,
				"utf8",
			);
			console.log(
				`${namespace}: wrote ${local.values.size}/${prose.length} local translation(s)`,
			);
			continue;
		}
		for (let index = 0; index < work.length; index++) {
			const batch = work[index];
			try {
				const values = translateBatch(batch, args.source, args.target);
				for (const entry of batch) {
					const value = values.get(entry.key);
					if (value === undefined) {
						throw new Error(`translation missing for ${entry.key}`);
					}
					setPath(targetTree, entry.key, value);
					translated++;
				}
			} catch (batchError) {
				for (const entry of batch) {
					try {
						const value = translateOne(entry, args.source, args.target);
						setPath(targetTree, entry.key, value);
						translated++;
					} catch (caught) {
						failures.push({
							...entry,
							reason: caught instanceof Error ? caught.message : String(caught),
						});
					}
				}
			}
			if ((index + 1) % 10 === 0 || index + 1 === work.length) {
				console.log(`${namespace}: ${index + 1}/${work.length} batches`);
			}
		}
		writeFileSync(
			targetPath,
			`${JSON.stringify(targetTree, null, "\t")}\n`,
			"utf8",
		);
	}

	console.log(
		`${args.apply ? "wrote" : "would write"} ${translated} translations and ${copied} exact technical copies`,
	);
	if (failures.length) {
		for (const failure of failures) {
			console.error(`${failure.key}: ${failure.reason}`);
		}
		throw new Error(`${failures.length} value(s) remain untranslated`);
	}
}

if (import.meta.main) main();
