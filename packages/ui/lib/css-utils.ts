import postcss, { type Declaration, type Root, type Rule } from "postcss";

/**
 * Branded type representing CSS that has been sanitized by safeScopedCss.
 * This marks the string as safe for use in dangerouslySetInnerHTML.
 */
export type SanitizedCSS = string & { readonly __sanitized: unique symbol };

/**
 * Sanitizes and scopes CSS using PostCSS for proper parsing.
 * This prevents XSS attacks like `</style>` injection that regex-based
 * approaches are vulnerable to.
 *
 * Removes dangerous constructs:
 * - expression() (IE)
 * - javascript:/vbscript: URLs
 * - behavior: property (IE)
 * - -moz-binding (Firefox)
 * - @import rules (can load external resources)
 * - @charset rules
 * - Invalid/unparseable CSS
 *
 * Scopes all selectors to the provided container (except @keyframes internals and :root).
 */

const DANGEROUS_PROPERTIES = new Set([
	"behavior",
	"-moz-binding",
	"-webkit-binding",
]);

const DANGEROUS_VALUE_PATTERNS = [
	/expression\s*\(/i,
	/javascript\s*:/i,
	/vbscript\s*:/i,
	/data\s*:\s*text\/html/i,
];

const BLOCKED_AT_RULES = new Set(["import", "charset"]);

const KNOWN_CORRUPTION_TOKENS = new Map([
	["__codex_directive_quoted_closing_brace__", "}"],
	["__codex_directive_escaped_double_quote__", '"'],
]);

function isDangerousValue(value: string): boolean {
	return DANGEROUS_VALUE_PATTERNS.some((pattern) => pattern.test(value));
}

function sanitizeDeclaration(decl: Declaration): void {
	// Remove dangerous properties entirely
	if (DANGEROUS_PROPERTIES.has(decl.prop.toLowerCase())) {
		decl.remove();
		return;
	}

	// Remove declarations with dangerous values
	if (isDangerousValue(decl.value)) {
		decl.remove();
		return;
	}

	// Sanitize url() values - allow safe schemes only
	if (decl.value.includes("url(")) {
		const urlMatch = decl.value.match(/url\s*\(\s*(['"]?)([^'")\s]+)\1\s*\)/gi);
		if (urlMatch) {
			for (const match of urlMatch) {
				const urlContent = match
					.replace(/url\s*\(\s*['"]?/i, "")
					.replace(/['"]?\s*\)$/i, "");
				// Block javascript:, vbscript:, and dangerous data: URLs
				if (
					/^(javascript|vbscript):/i.test(urlContent) ||
					/^data:text\/html/i.test(urlContent)
				) {
					decl.remove();
					return;
				}
			}
		}
	}
}

export interface SafeScopedCssOptions {
	/**
	 * Treat `:root` as the scoped container instead of the document root.
	 * This is useful for self-contained surfaces that expose CSS variables.
	 */
	scopeRoot?: boolean;
}

function scopeSelectorForRule(
	selector: string,
	scope: string,
	options: SafeScopedCssOptions,
): string {
	const trimmed = selector.trim();

	// Don't scope empty selectors
	if (!trimmed) {
		return trimmed;
	}

	// Don't scope keyframe percentages
	if (/^\d+%$/.test(trimmed) || trimmed === "from" || trimmed === "to") {
		return trimmed;
	}

	// Self-contained surfaces can use :root as an ergonomic alias for their
	// own root without leaking variables into the rest of the application.
	if (trimmed.startsWith(":root")) {
		return options.scopeRoot ? trimmed.replace(/^:root/, scope) : trimmed;
	}

	// Replace body/html with the scope selector itself (these are "root" selectors for the page)
	// Also handle combinations like "body.dark" → "[scope].dark"
	if (/^(body|html)($|[.#:\[])/.test(trimmed)) {
		return trimmed.replace(/^(body|html)/, scope);
	}

	// For everything else, prefix with scope
	return `${scope} ${trimmed}`;
}

function inheritsSelectorScope(rule: Rule): boolean {
	let parent: Rule["parent"] | Root["parent"] = rule.parent;
	while (parent) {
		// Nested selectors, including implicit descendants, inherit the scoped
		// parent. Prefixing them again asks for a second canvas inside that parent.
		if (parent.type === "rule") return true;
		if (
			parent.type === "atrule" &&
			/^(?:-webkit-)?keyframes$/i.test(parent.name)
		) {
			return true;
		}
		parent = parent.parent;
	}
	return false;
}

function normalizeCssInput(css: string): string {
	let normalizedCss = css.trim();

	// Fix double-encoded JSON strings (legacy data corruption).
	if (normalizedCss.startsWith('"') && normalizedCss.endsWith('"')) {
		try {
			const parsed = JSON.parse(normalizedCss);
			if (typeof parsed === "string") {
				normalizedCss = parsed.trim();
			}
		} catch {
			// Not valid JSON, continue with the original CSS.
		}
	}

	// Some generated A2UI payloads have leaked these transport sentinels into
	// customCss. They have a single unambiguous CSS meaning, so repair them
	// before parsing rather than discarding the complete stylesheet.
	for (const [token, replacement] of KNOWN_CORRUPTION_TOKENS) {
		normalizedCss = normalizedCss.replaceAll(token, replacement);
	}

	return normalizedCss;
}

/**
 * Split CSS into complete top-level blocks in one pass. This is only used
 * after parsing the complete input failed, allowing valid rules around a
 * malformed or incomplete rule to survive without attempting to invent CSS.
 */
function splitCompleteTopLevelBlocks(css: string): string[] {
	const blocks: string[] = [];
	let start = 0;
	let braceDepth = 0;
	let parenDepth = 0;
	let quote: '"' | "'" | undefined;
	let escaped = false;
	let inComment = false;

	for (let index = 0; index < css.length; index += 1) {
		const character = css[index];
		const next = css[index + 1];

		if (inComment) {
			if (character === "*" && next === "/") {
				inComment = false;
				index += 1;
			}
			continue;
		}

		if (quote) {
			if (escaped) {
				escaped = false;
				continue;
			}
			if (character === "\\") {
				escaped = true;
				continue;
			}
			if (character === quote) quote = undefined;
			continue;
		}

		if (character === "/" && next === "*") {
			inComment = true;
			index += 1;
			continue;
		}
		if (character === '"' || character === "'") {
			quote = character;
			continue;
		}
		if (character === "(") {
			parenDepth += 1;
			continue;
		}
		if (character === ")") {
			parenDepth = Math.max(0, parenDepth - 1);
			continue;
		}
		if (character === "{") {
			braceDepth += 1;
			continue;
		}
		if (character === "}") {
			braceDepth = Math.max(0, braceDepth - 1);
			if (braceDepth === 0) {
				blocks.push(css.slice(start, index + 1));
				start = index + 1;
			}
			continue;
		}

		// Preserve complete top-level statement at-rules such as @import so the
		// normal sanitizer can remove them. Semicolons inside functions do not
		// delimit a top-level block.
		if (character === ";" && braceDepth === 0 && parenDepth === 0) {
			blocks.push(css.slice(start, index + 1));
			start = index + 1;
		}
	}

	return blocks.filter((block) => block.trim().length > 0);
}

function parseCssBestEffort(css: string): {
	root: Root | null;
	recoveredBlocks: number;
	totalBlocks: number;
} {
	try {
		return {
			root: postcss.parse(css),
			recoveredBlocks: 0,
			totalBlocks: 0,
		};
	} catch {
		const blocks = splitCompleteTopLevelBlocks(css);
		if (blocks.length === 0) {
			return { root: null, recoveredBlocks: 0, totalBlocks: 0 };
		}

		const recoveredRoot = postcss.root();
		let recoveredBlocks = 0;
		for (const block of blocks) {
			try {
				const parsedBlock = postcss.parse(block);
				recoveredRoot.append(parsedBlock.nodes);
				recoveredBlocks += 1;
			} catch {
				// Keep processing later independent blocks.
			}
		}

		return {
			root: recoveredBlocks > 0 ? recoveredRoot : null,
			recoveredBlocks,
			totalBlocks: blocks.length,
		};
	}
}

/**
 * Safely scopes and sanitizes CSS for injection using PostCSS.
 * This is the primary function to use for user-provided CSS.
 *
 * @param css - The CSS string to process
 * @param scopeSelector - The attribute selector for scoping (e.g., '[data-page-id="abc"]')
 * @param options - Optional scoping behavior for self-contained surfaces
 * @returns Safe, scoped CSS ready for injection. Returns an empty string only
 * when no complete, parseable CSS blocks can be recovered.
 */
export function safeScopedCss(
	css: string,
	scopeSelector: string,
	options: SafeScopedCssOptions = {},
): SanitizedCSS {
	if (!css || typeof css !== "string") {
		return "" as SanitizedCSS;
	}

	// Trim whitespace and repair narrowly identified transport corruption.
	const trimmedCss = normalizeCssInput(css);
	if (!trimmedCss) {
		return "" as SanitizedCSS;
	}

	const { root, recoveredBlocks, totalBlocks } = parseCssBestEffort(trimmedCss);
	if (!root) {
		console.warn(
			`[safeScopedCss] Invalid CSS could not be recovered. First 200 chars: ${trimmedCss.slice(0, 200)}`,
		);
		return "" as SanitizedCSS;
	}
	if (recoveredBlocks > 0) {
		console.warn(
			`[safeScopedCss] Recovered ${recoveredBlocks} of ${totalBlocks} complete top-level CSS blocks after the full stylesheet failed to parse.`,
		);
	}

	// Sanitize every nesting level before scoping each independent selector.
	root.walkAtRules((atRule) => {
		if (BLOCKED_AT_RULES.has(atRule.name.toLowerCase())) atRule.remove();
	});
	root.walkDecls(sanitizeDeclaration);
	root.walkRules((rule) => {
		if (inheritsSelectorScope(rule)) return;
		rule.selectors = rule.selectors.map((selector) =>
			scopeSelectorForRule(selector, scopeSelector, options),
		);
	});

	return root.toString() as SanitizedCSS;
}

/**
 * Creates props for a style element with sanitized CSS.
 * This helper ensures dangerouslySetInnerHTML is only used with sanitized content.
 * The CSS MUST be sanitized by safeScopedCss before being passed here.
 */
export function createSanitizedStyleProps(sanitizedCss: SanitizedCSS): {
	dangerouslySetInnerHTML: { __html: string };
} {
	// Security: This function only accepts SanitizedCSS type which can only be
	// produced by safeScopedCss, ensuring the CSS has been properly sanitized.
	return {
		dangerouslySetInnerHTML: { __html: sanitizedCss },
	};
}
