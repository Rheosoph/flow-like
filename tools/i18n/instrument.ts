/**
 * Wraps `i18next-cli instrument` so its output actually compiles.
 *
 * The CLI finds hardcoded strings well and rewrites them to `t('key', 'Default')`,
 * but it only adds `import i18next from 'i18next'` — nothing ever binds `t`, so
 * every touched file fails to build. This pass fixes that:
 *
 *   - drops the unused `i18next` default import
 *   - adds `import { useTranslation } from "@flow-like/locales"`
 *   - inserts `const { t } = useTranslation("<ns>")` at the top of every React
 *     component that ended up containing a `t()` call
 *
 * Calls that are not inside a component (module-scope helpers, class methods,
 * plain functions) cannot use a hook. Those are rewritten to `i18next.t(...)`
 * and the import is kept, which is correct — they just do not re-render on a
 * language switch, so they are reported for review.
 *
 * Usage:
 *   bun tools/i18n/instrument.ts --namespace settings --input "packages/ui/components/settings/**\/*.tsx"
 *   bun tools/i18n/instrument.ts --namespace store --input "..." --dry-run
 */
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";

const REPO_ROOT = path.resolve(import.meta.dirname, "../..");
const HOOK_IMPORT = 'import { useTranslation } from "@flow-like/locales";';

interface Args {
	namespace: string;
	input: string[];
	dryRun: boolean;
	/** Re-run only the repair passes over files the CLI already rewrote. */
	fixOnly: boolean;
	/** Path of a file to un-instrument completely. */
	revert?: string;
}

function parseArgs(argv: string[]): Args {
	const args: Args = {
		namespace: "common",
		input: [],
		dryRun: false,
		fixOnly: false,
		revert: undefined,
	};
	for (let i = 0; i < argv.length; i++) {
		if (argv[i] === "--namespace") args.namespace = argv[++i];
		else if (argv[i] === "--input") args.input.push(argv[++i]);
		else if (argv[i] === "--dry-run") args.dryRun = true;
		else if (argv[i] === "--fix-only") args.fixOnly = true;
		else if (argv[i] === "--revert") args.revert = argv[++i];
	}
	if (args.input.length === 0 && !args.fixOnly && !args.revert) {
		throw new Error("at least one --input glob is required");
	}
	return args;
}

/**
 * The CLI instruments any string literal that looks like text, which sweeps in a
 * lot that is not: Tailwind class lists, CSS values, chart config, hex colours,
 * camelCase property names. Translating those breaks rendering outright, so the
 * bar for "this is prose" has to be high.
 *
 * A string qualifies only if, once placeholders are removed, it reads like
 * language: it has a word of two or more letters and shows none of the
 * signatures below.
 */
const CODE_SIGNATURES = [
	/^#[0-9a-fA-F]{3,8}$/, // hex colour
	/\b(rgba?|hsla?|calc|var|url|translate[XYZ]?|scale|rotate)\(/, // CSS function
	/^-?\d+(\.\d+)?(px|rem|em|vh|vw|%|deg|ms|s)\b/, // a measurement
	/^[a-z]+(-[a-z0-9]+)+$/, // kebab identifier / single utility class
	/^[a-z]+[A-Z][a-zA-Z0-9]*$/, // camelCase identifier, e.g. alignItems
	/^[A-Z][A-Z0-9_]{2,}$/, // SCREAMING_CASE constant or HTTP verb
	/^data:|^https?:\/\/|^\/[a-z0-9-]+\//i, // URI or path
	/^\{.*\}$|^\[.*\]$/s, // JSON-ish blob
	/^```|```$/, // fenced source or command snippet
	/^(?:curl|wget|brew|winget|choco|npm|bun|pnpm|yarn|git|sudo|docker|kubectl|npx|mise)\s/i,
	/^(?:authorization\s*:\s*)?bearer(?:\s|$)/i,
	/<!doctype\s+html|<html\b|<link\b|<style\b/i,
];

/**
 * Interpolation does not make a machine value safe to translate. These shapes
 * are commonly assembled from runtime values, and changing their punctuation,
 * casing or whitespace alters URLs, IDs, source code, CSS and wire formats.
 */
const INTERPOLATED_CODE_SIGNATURES = [
	/^https?:\/\//i,
	/^\/?\{\{[^}]+\}\}(?:[/:?&#._=-]|$)/, // URL/path/ID beginning with a token
	/^\{\{[^}]+\}\}(?:[/:?&#._=-]|\{\{)/, // composed IDs, colours and durations
	/\b(?:rgb|rgba|hsl|hsla|calc|var|url)\s*\(/i,
	/\b(?:cast|select|insert|update|delete|create|alter)\s*[({]/i,
	/<!doctype\s+html|<html\b|<link\b|<style\b/i,
	/^\s*(?:\[(?!\[)|\{(?!\{)).*[\]}]\s*$/s,
	/(?:^|\s)(?:curl|wget|brew|npm|bun|pnpm|yarn|git|sudo|docker|kubectl|npx|mise)\s/i,
	/^\s*(?:const|let|var|return|import|export)\b/,
	/\bfrom\s+["'][^"']+["']/,
	/\bnew\s+[A-Z_$][\w$]*\s*\(/,
	/^[A-Za-z_$][\w.$-]*=\{\{[^}]+\}\}$/,
	/^(?:authorization\s*:\s*)?bearer\s+\{\{[^}]+\}\}$/i,
	/^\{\{[^}]+\}\}(?:px|rem|em|vh|vw|%|deg|ms|s|m|h|d|b|kb|mb|gb)$/i,
	/^[-\w/.:@]+\{\{[^}]+\}\}[-\w/.:@{}?&=]*$/,
];

/** Tailwind-ish: every token is lowercase and carries a `-`, `/`, `:` or `[`. */
function looksLikeClassList(text: string): boolean {
	const tokens = text.trim().split(/\s+/);
	if (tokens.length < 2) return false;
	return tokens.every(
		(token) =>
			/^[a-z0-9][a-z0-9:.\/\[\]%_-]*$/.test(token) && /[-:/[]/.test(token),
	);
}

export function isProse(defaultValue: string): boolean {
	const hasPlaceholder = /\{\{[^}]*\}\}/.test(defaultValue);
	const bare = defaultValue.replace(/\{\{[^}]*\}\}/g, " ").trim();
	if (!bare) return false;
	if (
		INTERPOLATED_CODE_SIGNATURES.some((pattern) => pattern.test(defaultValue))
	) {
		return false;
	}
	if (!/\p{L}{2,}/u.test(bare)) return false;
	if (looksLikeClassList(bare)) return false;
	// The shape checks below judge a whole literal. A string built around a
	// placeholder ("{{count}} failing") is a composed message, and its leftover
	// fragment will often look like a bare identifier — exempt it.
	if (CODE_SIGNATURES.some((pattern) => pattern.test(bare))) {
		return false;
	}

	// Real sentences and labels contain letters far more than punctuation.
	const letters = (bare.match(/\p{L}/gu) ?? []).length;
	const symbols = (bare.match(/[^\p{L}\p{N}\s'’.,!?:;()&%°—–-]/gu) ?? [])
		.length;
	if (symbols > 0 && letters / symbols < 3) return false;

	return true;
}

/** `t('k', '{{a}}.0', { a: x })` → `` `${x}.0` `` */
function rebuildTemplate(
	call: ts.CallExpression,
	source: ts.SourceFile,
): string | undefined {
	const [, defaultArg, optionsArg] = call.arguments;
	if (!defaultArg || !ts.isStringLiteral(defaultArg)) return undefined;

	const values = new Map<string, string>();
	if (optionsArg && ts.isObjectLiteralExpression(optionsArg)) {
		for (const property of optionsArg.properties) {
			if (ts.isShorthandPropertyAssignment(property)) {
				values.set(property.name.text, property.name.text);
				continue;
			}
			if (!ts.isPropertyAssignment(property)) return undefined;
			const name = property.name.getText(source).replace(/^["']|["']$/g, "");
			values.set(name, property.initializer.getText(source));
		}
	}

	let unresolved = false;
	// Escape before substituting, so only the placeholders we inject stay live.
	const body = defaultArg.text
		.replace(/[\\`]/g, "\\$&")
		.replace(/\$\{/g, "\\${")
		.replace(/\{\{\s*([^}]+?)\s*\}\}/g, (_, token) => {
			const expression = values.get(token);
			if (expression === undefined) {
				unresolved = true;
				return "";
			}
			return `\${${expression}}`;
		});
	if (unresolved) return undefined;
	return `\`${body}\``;
}

/**
 * JSX text keeps the source indentation of the line it wrapped onto, so the CLI
 * captures defaults like `"and\n\t\t\t\tremove it"`. HTML collapses that at
 * render time, but it makes the locale files unreadable and gives translators
 * strings full of tabs. Collapse it to what the user actually sees.
 */
const ENTITIES: Record<string, string> = {
	"&amp;": "&",
	"&lt;": "<",
	"&gt;": ">",
	"&quot;": '"',
	"&apos;": "'",
	"&nbsp;": "\u00a0",
	"&mdash;": "—",
	"&ndash;": "–",
	"&hellip;": "…",
};

function collapse(text: string): string {
	// JSX entities are decoded by the renderer, so a translator must never see
	// `&amp;` — they would faithfully translate the markup along with the word.
	const decoded = text.replace(
		/&(amp|lt|gt|quot|apos|nbsp|mdash|ndash|hellip);/g,
		(entity) => ENTITIES[entity] ?? entity,
	);
	return decoded.replace(/\s+/g, " ").trim();
}

/** A function node is treated as a component if its name is capitalised. */
function componentName(node: ts.Node): string | undefined {
	if (ts.isFunctionDeclaration(node) && node.name) return node.name.text;
	if (
		(ts.isArrowFunction(node) || ts.isFunctionExpression(node)) &&
		ts.isVariableDeclaration(node.parent) &&
		ts.isIdentifier(node.parent.name)
	) {
		return node.parent.name.text;
	}
	if (ts.isFunctionExpression(node) && node.name) return node.name.text;
	return undefined;
}

function isComponent(node: ts.Node): boolean {
	const name = componentName(node);
	return name !== undefined && /^[A-Z]/.test(name);
}

interface Insertion {
	/** Offset just inside the component body's opening brace. */
	pos: number;
	indent: string;
}

interface FileResult {
	file: string;
	hooksInserted: number;
	/** Format templates the CLI mistook for prose, rebuilt as template literals. */
	reverted: number;
	/** Default values whose JSX indentation was collapsed. */
	collapsed: number;
	fallbacks: { line: number; text: string }[];
	skipped?: string;
}

function processFile(
	file: string,
	namespace: string,
	dryRun: boolean,
): FileResult {
	const original = readFileSync(file, "utf8");
	const result: FileResult = {
		file,
		hooksInserted: 0,
		reverted: 0,
		collapsed: 0,
		fallbacks: [],
	};

	if (
		!original.includes("import i18next from 'i18next'") &&
		!original.includes("i18next.t(") &&
		!original.includes('from "@flow-like/locales"')
	) {
		result.skipped = "no instrument output — nothing to fix";
		return result;
	}

	const source = ts.createSourceFile(
		file,
		original,
		ts.ScriptTarget.Latest,
		true,
		ts.ScriptKind.TSX,
	);

	// A file may already pull the hook in for its own strings.
	const alreadyImportsHook =
		/useTranslation\s*}?\s*from\s*"@flow-like\/locales"/.test(original);

	const insertions: Insertion[] = [];
	const seenComponents = new Set<ts.Node>();
	const fallbackCalls: ts.CallExpression[] = [];

	function nearestComponent(node: ts.Node): ts.Node | undefined {
		let cursor: ts.Node | undefined = node.parent;
		while (cursor) {
			if (
				(ts.isFunctionDeclaration(cursor) ||
					ts.isArrowFunction(cursor) ||
					ts.isFunctionExpression(cursor) ||
					ts.isMethodDeclaration(cursor)) &&
				isComponent(cursor)
			) {
				return cursor;
			}
			cursor = cursor.parent;
		}
		return undefined;
	}

	const nonProse: { call: ts.CallExpression; replacement: string }[] = [];
	const rewrites: { start: number; end: number; text: string }[] = [];

	function visit(node: ts.Node): void {
		if (
			ts.isCallExpression(node) &&
			(ts.isIdentifier(node.expression)
				? node.expression.text === "t"
				: node.expression.getText(source) === "i18next.t")
		) {
			const defaultArg = node.arguments[1];
			if (defaultArg && ts.isStringLiteral(defaultArg)) {
				if (!isProse(defaultArg.text)) {
					const replacement =
						rebuildTemplate(node, source) ??
						(node.arguments.length === 2
							? JSON.stringify(defaultArg.text)
							: undefined);
					if (replacement) {
						nonProse.push({ call: node, replacement });
						return;
					}
				}
				const collapsed = collapse(defaultArg.text);
				if (collapsed !== defaultArg.text) {
					rewrites.push({
						start: defaultArg.getStart(source),
						end: defaultArg.getEnd(),
						text: JSON.stringify(collapsed),
					});
				}
			}

			// Plural variants land in an options object as defaultValue_one /
			// defaultValue_other and need the same treatment.
			const optionsArg = node.arguments[1];
			if (optionsArg && ts.isObjectLiteralExpression(optionsArg)) {
				for (const property of optionsArg.properties) {
					if (
						!ts.isPropertyAssignment(property) ||
						!ts.isStringLiteral(property.initializer) ||
						!/^defaultValue/.test(property.name.getText(source))
					) {
						continue;
					}
					const collapsed = collapse(property.initializer.text);
					if (collapsed !== property.initializer.text) {
						rewrites.push({
							start: property.initializer.getStart(source),
							end: property.initializer.getEnd(),
							text: JSON.stringify(collapsed),
						});
					}
				}
			}
		}

		if (
			ts.isCallExpression(node) &&
			ts.isIdentifier(node.expression) &&
			node.expression.text === "t"
		) {
			const owner = nearestComponent(node);
			if (!owner) {
				fallbackCalls.push(node);
			} else if (!seenComponents.has(owner)) {
				seenComponents.add(owner);
				const body = (owner as ts.FunctionLikeDeclaration).body;
				if (body && ts.isBlock(body)) {
					// Skip components that already destructure `t` themselves.
					const declaresT = body.statements.some(
						(statement) =>
							ts.isVariableStatement(statement) &&
							statement.declarationList.declarations.some((declaration) =>
								/(^|[^A-Za-z])t([^A-Za-z]|$)/.test(declaration.name.getText()),
							),
					);
					if (!declaresT) {
						const line = source.getLineAndCharacterOfPosition(
							body.statements[0]?.getStart(source) ?? body.getStart(source) + 1,
						);
						const lineStart = source.getPositionOfLineAndCharacter(
							line.line,
							0,
						);
						const indent =
							original
								.slice(lineStart, lineStart + line.character)
								.match(/^[\t ]*/)?.[0] ?? "\t";
						insertions.push({ pos: body.getStart(source) + 1, indent });
					}
				}
			}
		}
		ts.forEachChild(node, visit);
	}

	visit(source);

	// Apply edits back-to-front so earlier offsets stay valid.
	let output = original;
	const edits: { start: number; end: number; text: string }[] = [];

	edits.push(...rewrites);
	result.collapsed = rewrites.length;

	for (const entry of nonProse) {
		edits.push({
			start: entry.call.getStart(source),
			end: entry.call.getEnd(),
			text: entry.replacement,
		});
		result.reverted++;
	}

	for (const call of fallbackCalls) {
		const start = call.expression.getStart(source);
		edits.push({ start, end: start + 1, text: "i18next.t" });
		const { line } = source.getLineAndCharacterOfPosition(start);
		result.fallbacks.push({
			line: line + 1,
			text: call.getText(source).slice(0, 90),
		});
	}

	for (const insertion of insertions) {
		edits.push({
			start: insertion.pos,
			end: insertion.pos,
			text: `\n${insertion.indent}const { t } = useTranslation("${namespace}");`,
		});
	}

	edits.sort((a, b) => b.start - a.start);
	for (const edit of edits) {
		output = output.slice(0, edit.start) + edit.text + output.slice(edit.end);
	}
	result.hooksInserted = insertions.length;

	// Fix the imports last, so the offsets above were computed against the
	// original text.
	const withoutRawI18nextImport = output.replace(
		/^import i18next from ['"]i18next['"];?\n/m,
		"",
	);
	const keepsI18next = /\bi18next\./.test(withoutRawI18nextImport);
	// The runtime configures and exports the same singleton that react-i18next
	// uses. Import it through @flow-like/locales so a direct call cannot evaluate
	// before that configuration module has run.
	output = withoutRawI18nextImport;
	// Merge into the single locales import rather than adding another one —
	// this pass is re-run repeatedly and must be idempotent.
	const needed = new Set<string>();
	if (/<Trans[\s>]/.test(output)) needed.add("Trans");
	if (/\buseTranslation\(/.test(output)) needed.add("useTranslation");
	if (keepsI18next) needed.add("i18n as i18next");

	const existing = output.match(
		/^import \{([^}]*)\} from "@flow-like\/locales";\n/m,
	);
	for (const name of existing?.[1].split(",") ?? []) {
		const imported = name.trim();
		if (imported && (keepsI18next || imported !== "i18n as i18next")) {
			needed.add(imported);
		}
	}
	if (needed.size > 0) {
		const line = `import { ${[...needed].sort().join(", ")} } from "@flow-like/locales";\n`;
		output = existing
			? output.replace(existing[0], line)
			: output.replace(/^(import .*\n)/m, (match) => `${line}${match}`);
	} else if (existing) {
		output = output.replace(existing[0], "");
	}

	if (!dryRun && output !== original) writeFileSync(file, output, "utf8");
	return result;
}

/**
 * Un-instruments a whole file: every `t('key', 'Default')` becomes the literal
 * again and the now-unused hook and import are removed. Needed for files whose
 * strings are not UI at all — LLM tool protocols, log lines, wire formats —
 * where a translation would change behaviour rather than presentation.
 */
function revertFile(file: string): number {
	const original = readFileSync(file, "utf8");
	const source = ts.createSourceFile(
		file,
		original,
		ts.ScriptTarget.Latest,
		true,
		ts.ScriptKind.TSX,
	);
	const edits: { start: number; end: number; text: string }[] = [];

	function visit(node: ts.Node): void {
		if (
			ts.isCallExpression(node) &&
			ts.isIdentifier(node.expression) &&
			node.expression.text === "t" &&
			node.arguments.length >= 2
		) {
			const [, defaultArg] = node.arguments;
			if (ts.isStringLiteral(defaultArg)) {
				const rebuilt = rebuildTemplate(node, source);
				edits.push({
					start: node.getStart(source),
					end: node.getEnd(),
					text: rebuilt ?? JSON.stringify(defaultArg.text),
				});
				return;
			}
			// Plural form: fall back to the `_other` variant.
			if (ts.isObjectLiteralExpression(defaultArg)) {
				const other = defaultArg.properties.find(
					(property) =>
						ts.isPropertyAssignment(property) &&
						property.name.getText(source).includes("defaultValue_other"),
				);
				if (
					other &&
					ts.isPropertyAssignment(other) &&
					ts.isStringLiteral(other.initializer)
				) {
					edits.push({
						start: node.getStart(source),
						end: node.getEnd(),
						text: JSON.stringify(other.initializer.text),
					});
					return;
				}
			}
		}
		ts.forEachChild(node, visit);
	}
	visit(source);

	let output = original;
	for (const edit of edits.sort((a, b) => b.start - a.start)) {
		output = output.slice(0, edit.start) + edit.text + output.slice(edit.end);
	}

	if (!/\bt\(/.test(output.replace(/useTranslation\(/g, ""))) {
		output = output.replace(
			/^\s*const \{ t \} = useTranslation\([^)]*\);\n/gm,
			"",
		);
	}
	if (!/useTranslation\(|<Trans[\s>]/.test(output)) {
		output = output.replace(
			/^import \{[^}]*\} from "@flow-like\/locales";\n/m,
			"",
		);
	}

	if (output !== original) writeFileSync(file, output, "utf8");
	return edits.length;
}

function main(): void {
	const args = parseArgs(process.argv.slice(2));

	if (args.revert) {
		const target = path.isAbsolute(args.revert)
			? args.revert
			: path.join(REPO_ROOT, args.revert);
		console.log(`==> reverted ${revertFile(target)} call(s) in ${args.revert}`);
		return;
	}
	const configPath = path.join(REPO_ROOT, ".i18n-instrument.config.ts");

	writeFileSync(
		configPath,
		`import { defineConfig } from "i18next-cli";
export default defineConfig({
	locales: ["en"],
	extract: {
		input: ${JSON.stringify(args.input)},
		ignore: ["**/node_modules/**", "**/.next/**", "**/out/**", "**/dist/**", "**/*.test.*", "**/__tests__/**"],
		output: ".i18n-instrument-scratch/{{language}}/{{namespace}}.json",
		defaultNS: "${args.namespace}",
	},
});
`,
		"utf8",
	);

	try {
		if (!args.fixOnly) {
			console.log(`==> instrumenting into namespace "${args.namespace}"`);
			execFileSync(
				path.join(REPO_ROOT, "tools/i18n/i18n.sh"),
				[
					"instrument",
					"-c",
					".i18n-instrument.config.ts",
					"--namespace",
					args.namespace,
					...(args.dryRun ? ["--dry-run"] : []),
				],
				{ cwd: REPO_ROOT, stdio: "inherit" },
			);
		}

		if (args.dryRun) return;

		// A normal instrumentation run only needs changed files. `--fix-only` is
		// also the migration repair/audit command, so it scans every source file:
		// already-modified files are not a reliable boundary after multiple passes.
		const candidateNames = args.fixOnly
			? execFileSync(
					"rg",
					[
						"--files",
						"apps/desktop",
						"apps/web",
						"packages/ui",
						"-g",
						"*.ts",
						"-g",
						"*.tsx",
					],
					{ cwd: REPO_ROOT, encoding: "utf8" },
				)
			: execFileSync("git", ["diff", "--name-only", "--diff-filter=M"], {
					cwd: REPO_ROOT,
					encoding: "utf8",
				});
		const touched = candidateNames
			.split("\n")
			.filter((line) => /\.tsx?$/.test(line))
			.map((line) => path.join(REPO_ROOT, line))
			.filter((file) => {
				if (!existsSync(file)) return false;
				const content = readFileSync(file, "utf8");
				return (
					content.includes("import i18next from 'i18next'") ||
					content.includes("i18next.t(") ||
					content.includes('from "@flow-like/locales"')
				);
			});

		console.log(`==> repairing ${touched.length} file(s)`);
		let hooks = 0;
		let reverted = 0;
		let collapsed = 0;
		const needsReview: FileResult[] = [];
		for (const file of touched) {
			const outcome = processFile(file, args.namespace, false);
			hooks += outcome.hooksInserted;
			reverted += outcome.reverted;
			collapsed += outcome.collapsed;
			if (outcome.fallbacks.length > 0) needsReview.push(outcome);
		}

		console.log(`==> inserted ${hooks} useTranslation hook(s)`);
		if (reverted > 0) {
			console.log(
				`==> reverted ${reverted} format template(s) the CLI mistook for prose`,
			);
		}
		if (collapsed > 0) {
			console.log(
				`==> collapsed JSX whitespace in ${collapsed} default value(s)`,
			);
		}
		if (needsReview.length > 0) {
			console.log("\n==> non-component call sites rewritten to i18next.t:");
			for (const outcome of needsReview) {
				const relative = path.relative(REPO_ROOT, outcome.file);
				if (outcome.skipped) {
					console.log(`  ${relative}: ${outcome.skipped}`);
					continue;
				}
				for (const fallback of outcome.fallbacks) {
					console.log(`  ${relative}:${fallback.line}  ${fallback.text}`);
				}
			}
		}
	} finally {
		rmSync(configPath, { force: true });
		rmSync(path.join(REPO_ROOT, ".i18n-instrument-scratch"), {
			recursive: true,
			force: true,
		});
	}
}

if (import.meta.main) main();
