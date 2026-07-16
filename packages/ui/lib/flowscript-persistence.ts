/**
 * Conservative FlowScript redaction for data that survives a renderer reload.
 *
 * This is intentionally not a FlowScript parser. A secret is persisted only when every
 * `@secret` marker belongs to a narrow variable-decorator stack ending in a single-line, typed
 * declaration. Primitive literal initializers are replaced; declarations without an initializer
 * are already safe. Anything more complex is omitted instead of trying to redact it with a
 * permissive regular expression.
 */

export type PersistableFlowScript =
	| {
			safe: true;
			source: string;
			redactedDeclarations: number;
			/** Unquoted string-initializer values that were redacted; used to scrub echoes elsewhere. */
			redactedLiterals: string[];
	  }
	| {
			safe: false;
			reason: string;
	  };

const SECRET_MARKER = /@secret\b/g;
const SECRET_ANNOTATION_LINE = /^\s*@secret\s*$/;
// FlowScript permits variable decorators in any order. The renderer currently emits category /
// description / schema before the flag decorators, but model-authored source commonly places
// `@secret` first. These are the only canonical standalone decorator lines we may safely step
// across while looking for the declaration owned by a secret marker; blank lines are parser-
// insignificant and safe to cross, while comments and unknown annotations remain hard boundaries.
const VARIABLE_DECORATOR_LINE =
	/^\s*@(readonly|runtime)\s*$|^\s*@(category|description|schema)\(\s*"(?:\\.|[^"\\])*"\s*\)\s*$/;
const SECRET_DECLARATION =
	/^(\s*(?:const|let|var)\s+[A-Za-z_$][\w$]*\s*:\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*)(.*?)\s*$/;
const SECRET_DECLARATION_WITHOUT_INITIALIZER =
	/^\s*(?:const|let|var)\s+[A-Za-z_$][\w$]*\s*:\s*(?:[A-Za-z_][A-Za-z0-9_]*(?:\s*\[\s*\])?|Map\s*<\s*string\s*,\s*[A-Za-z_][A-Za-z0-9_]*\s*>|Set\s*<\s*[A-Za-z_][A-Za-z0-9_]*\s*>)\s*;?(?:\s+\/\/@v:[A-Za-z0-9_-]+)?\s*$/;
const VARIABLE_ANCHOR_SUFFIX = /^(.*?)(\s+\/\/@v:[A-Za-z0-9_-]+)\s*$/;
const STRING_LITERAL = /^(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')$/;
const NUMBER_LITERAL = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/;

function safeInitializer(
	typeName: string,
	initializer: string,
): string | undefined {
	const normalizedType = typeName.toLowerCase();
	if (normalizedType === "string") {
		return STRING_LITERAL.test(initializer) ? '""' : undefined;
	}
	if (
		[
			"int",
			"integer",
			"float",
			"double",
			"number",
			"i32",
			"i64",
			"u32",
			"u64",
		].includes(normalizedType)
	) {
		return NUMBER_LITERAL.test(initializer) ? "0" : undefined;
	}
	if (normalizedType === "bool" || normalizedType === "boolean") {
		return /^(?:true|false)$/i.test(initializer) ? "false" : undefined;
	}
	return undefined;
}

/**
 * Return source that is safe to persist locally, or refuse the entire document.
 *
 * Refusal is important: partially redacting a multiline/template/call initializer could leave the
 * remaining lines in storage. The caller should retain that candidate in memory only.
 */
export function sanitizeFlowScriptForPersistence(
	source: string,
): PersistableFlowScript {
	if (source.includes("\0")) {
		return { safe: false, reason: "FlowScript contains a NUL byte." };
	}

	const markerCount = [...source.matchAll(SECRET_MARKER)].length;
	if (markerCount === 0) {
		return {
			safe: true,
			source,
			redactedDeclarations: 0,
			redactedLiterals: [],
		};
	}

	const newline = source.includes("\r\n") ? "\r\n" : "\n";
	const lines = source.split(/\r?\n/);
	let redactedDeclarations = 0;
	const redactedLiterals: string[] = [];
	for (let index = 0; index < lines.length; index += 1) {
		const line = lines[index] ?? "";
		if (!line.includes("@secret")) continue;
		if (!SECRET_ANNOTATION_LINE.test(line)) {
			return {
				safe: false,
				reason: `Unsupported @secret annotation at line ${index + 1}.`,
			};
		}

		let declarationIndex = index + 1;
		while (
			declarationIndex < lines.length &&
			((lines[declarationIndex] ?? "").trim() === "" ||
				VARIABLE_DECORATOR_LINE.test(lines[declarationIndex] ?? ""))
		) {
			declarationIndex += 1;
		}
		const declaration = lines[declarationIndex];
		if (declaration === undefined) {
			return {
				safe: false,
				reason: `@secret at line ${index + 1} has no declaration.`,
			};
		}
		if (SECRET_DECLARATION_WITHOUT_INITIALIZER.test(declaration)) {
			redactedDeclarations += 1;
			index = declarationIndex;
			continue;
		}

		const match = declaration.match(SECRET_DECLARATION);
		if (!match) {
			return {
				safe: false,
				reason: `Unsupported @secret declaration at line ${declarationIndex + 1}.`,
			};
		}

		let initializer = (match[3] ?? "").trim();
		let anchor = "";
		const anchorMatch = initializer.match(VARIABLE_ANCHOR_SUFFIX);
		if (anchorMatch) {
			initializer = (anchorMatch[1] ?? "").trimEnd();
			anchor = anchorMatch[2] ?? "";
		}
		const hasSemicolon = initializer.endsWith(";");
		if (hasSemicolon) initializer = initializer.slice(0, -1).trimEnd();
		const replacement = safeInitializer(match[2] ?? "", initializer);
		if (replacement === undefined) {
			return {
				safe: false,
				reason: `Non-literal or unsupported @secret initializer at line ${declarationIndex + 1}.`,
			};
		}
		if (STRING_LITERAL.test(initializer)) {
			const inner = initializer.slice(1, -1);
			const unescaped = inner.replace(/\\(.)/g, "$1");
			redactedLiterals.push(unescaped);
			if (inner !== unescaped) redactedLiterals.push(inner);
		}

		lines[declarationIndex] =
			`${match[1]}${replacement}${hasSemicolon ? ";" : ""}${anchor}`;
		redactedDeclarations += 1;
		index = declarationIndex;
	}

	if (redactedDeclarations !== markerCount) {
		return {
			safe: false,
			reason: "Not every @secret marker could be matched to one declaration.",
		};
	}

	return {
		safe: true,
		source: lines.join(newline),
		redactedDeclarations,
		redactedLiterals,
	};
}

const REDACTION_NOTICE_PREFIX =
	"[REDACTED] Persisted FlowScript copy omitted for secret safety (not a parser/reconcile error):";
const SOURCE_LIKE_KEY =
	/^(?:source|flowscript|script|content|retained_full_source)$/i;
// MCP-style envelopes nest the actual payload under `text` inside a source-like container
// (`content: [{ type: "text", text: <source> }]`). The key only inherits source-likeness; it is
// not source-like on its own.
const TEXT_ENVELOPE_KEY = /^text$/i;
const MAX_NESTED_JSON_SANITIZE_DEPTH = 8;
const SCRUBBED_SECRET_PLACEHOLDER = "[REDACTED]";
const MIN_SCRUBBED_LITERAL_LENGTH = 5;

function redactionNotice(reason: string): string {
	// The notice must never contain the literal secret-annotation trigger substring: a repeated
	// sanitizer pass would otherwise treat the notice itself as secret-bearing text and re-redact
	// it, corrupting the reported reason (observed as a wrongly re-reported line number).
	return `${REDACTION_NOTICE_PREFIX} ${reason.replaceAll("@secret", "(at)secret")}`;
}

function parseJsonContainer(text: string): unknown {
	const trimmed = text.trim();
	if (
		!(
			(trimmed.startsWith("{") && trimmed.endsWith("}")) ||
			(trimmed.startsWith("[") && trimmed.endsWith("]"))
		)
	) {
		return undefined;
	}
	try {
		return JSON.parse(trimmed);
	} catch {
		return undefined;
	}
}

function collectRedactedLiterals(
	literals: readonly string[],
	collected: Set<string>,
): void {
	for (const literal of literals) {
		if (literal.length >= MIN_SCRUBBED_LITERAL_LENGTH) collected.add(literal);
	}
}

/**
 * Replace every occurrence of a redacted `@secret` initializer value that was echoed into other
 * fields (diagnostics `actual`, error messages, …) of the same document. Every JSON escape level
 * up to the nested-JSON depth limit is scrubbed — an echo inside a doubly nested envelope
 * (`content` → `text` → payload) appears multiply escaped in the serialized document and must not
 * survive. Deeper (longer) forms are replaced first so a shallow match cannot split a deep one.
 */
function scrubCollectedLiterals(text: string, collected: Set<string>): string {
	let output = text;
	for (const literal of collected) {
		const forms: string[] = [];
		let form = literal;
		for (let level = 0; level <= MAX_NESTED_JSON_SANITIZE_DEPTH; level += 1) {
			forms.push(form);
			const escaped = JSON.stringify(form).slice(1, -1);
			if (escaped === form) break;
			form = escaped;
		}
		for (let index = forms.length - 1; index >= 0; index -= 1) {
			output = output.replaceAll(
				forms[index] ?? "",
				SCRUBBED_SECRET_PLACEHOLDER,
			);
		}
	}
	return output;
}

function sanitizeJsonValue(
	value: unknown,
	sourceLike: boolean,
	depth: number,
	collected: Set<string>,
): unknown {
	if (typeof value === "string") {
		// Content-based, key-independent: ANY string carrying an @secret marker is sanitized,
		// whatever field it sits in (patch old_text/new_text, message, error, summary, …). The
		// source-like key list is only an optimization hint for secret-free source fields, never
		// an exemption. Secret-free non-source strings pass through untouched.
		if (sourceLike || value.includes("@secret")) {
			return sanitizeTextInternal(value, depth + 1, collected);
		}
		return value;
	}
	if (Array.isArray(value)) {
		return value.map((entry) =>
			sanitizeJsonValue(entry, sourceLike, depth + 1, collected),
		);
	}
	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value).map(([key, entry]) => [
				key,
				sanitizeJsonValue(
					entry,
					SOURCE_LIKE_KEY.test(key) ||
						(sourceLike && TEXT_ENVELOPE_KEY.test(key)),
					depth + 1,
					collected,
				),
			]),
		);
	}
	return value;
}

function sanitizeTextInternal(
	text: string,
	depth: number,
	collected: Set<string>,
): string {
	// No prefix-based short-circuit: a well-formed redaction notice never contains the literal
	// `@secret` trigger (redactionNotice rewrites it), so it is covered by the marker check below.
	// Text that merely STARTS with the notice but still carries `@secret` (legacy notices,
	// `<notice> + @secret …` concatenations) must be re-scanned, not passed through verbatim.
	if (!text.includes("@secret")) return text;
	if (depth < MAX_NESTED_JSON_SANITIZE_DEPTH) {
		const container = parseJsonContainer(text);
		if (container !== null && typeof container === "object") {
			try {
				return JSON.stringify(
					sanitizeJsonValue(container, false, depth, collected),
				);
			} catch {
				// Fall through to the fail-closed plain-text path.
			}
		}
	}
	const sanitized = sanitizeFlowScriptForPersistence(text);
	if (!sanitized.safe) return redactionNotice(sanitized.reason);
	collectRedactedLiterals(sanitized.redactedLiterals, collected);
	return sanitized.source;
}

/**
 * Safe text for persisted plan/debug fields.
 *
 * JSON tool payloads are sanitized structurally and content-based: every string field carrying an
 * `@secret` marker is redacted regardless of its key, while secret-free sibling evidence (status,
 * diagnostics, review notes, revision) survives verbatim. Redacted string-initializer values are
 * additionally scrubbed from every other field of the same document so echoed secrets
 * (diagnostics `actual`, error messages) cannot leak. Idempotent: sanitized output passes through
 * unchanged. Genuinely unparseable secret-bearing plain text is still omitted fail-closed.
 */
export function sanitizePotentialFlowScriptTextForPersistence(
	text: string,
): string {
	const collected = new Set<string>();
	const sanitized = sanitizeTextInternal(text, 0, collected);
	return scrubCollectedLiterals(sanitized, collected);
}
