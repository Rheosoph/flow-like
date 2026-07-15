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
		return { safe: true, source, redactedDeclarations: 0 };
	}

	const newline = source.includes("\r\n") ? "\r\n" : "\n";
	const lines = source.split(/\r?\n/);
	let redactedDeclarations = 0;
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
	};
}

/** Safe text for persisted plan/debug fields. Unsupported secret-bearing text is omitted. */
export function sanitizePotentialFlowScriptTextForPersistence(
	text: string,
): string {
	if (!text.includes("@secret")) return text;
	const sanitized = sanitizeFlowScriptForPersistence(text);
	return sanitized.safe
		? sanitized.source
		: `[REDACTED] Persisted FlowScript copy omitted for secret safety (not a parser/reconcile error): ${sanitized.reason}`;
}
