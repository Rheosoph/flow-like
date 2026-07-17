import postcss, {
	type AtRule,
	type Declaration,
	type Root,
	type Rule,
} from "postcss";

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

function scopeSelector(
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

function processRule(
	rule: Rule,
	scope: string,
	insideKeyframes: boolean,
	options: SafeScopedCssOptions,
): void {
	// Sanitize all declarations
	rule.walkDecls((decl) => sanitizeDeclaration(decl));

	// Scope selectors (unless inside @keyframes)
	if (!insideKeyframes) {
		rule.selectors = rule.selectors.map((sel) =>
			scopeSelector(sel, scope, options),
		);
	}
}

function processAtRule(
	atRule: AtRule,
	scope: string,
	options: SafeScopedCssOptions,
): void {
	const name = atRule.name.toLowerCase();

	// Remove blocked at-rules
	if (BLOCKED_AT_RULES.has(name)) {
		atRule.remove();
		return;
	}

	// For keyframes, sanitize but don't scope the internal selectors
	if (name === "keyframes") {
		atRule.walkRules((rule) => processRule(rule, scope, true, options));
		atRule.walkDecls((decl) => sanitizeDeclaration(decl));
		return;
	}

	// For media, supports, container, layer - scope the internal rules
	if (["media", "supports", "container", "layer"].includes(name)) {
		atRule.walkRules((rule) => processRule(rule, scope, false, options));
		atRule.walkDecls((decl) => sanitizeDeclaration(decl));
		return;
	}

	// For other at-rules (like @font-face), just sanitize declarations
	atRule.walkDecls((decl) => sanitizeDeclaration(decl));
}

/**
 * Safely scopes and sanitizes CSS for injection using PostCSS.
 * This is the primary function to use for user-provided CSS.
 *
 * @param css - The CSS string to process
 * @param scopeSelector - The attribute selector for scoping (e.g., '[data-page-id="abc"]')
 * @param options - Optional scoping behavior for self-contained surfaces
 * @returns Safe, scoped CSS ready for injection. Returns empty string on parse errors.
 */
export function safeScopedCss(
	css: string,
	scopeSelector: string,
	options: SafeScopedCssOptions = {},
): SanitizedCSS {
	if (!css || typeof css !== "string") {
		return "" as SanitizedCSS;
	}

	// Trim whitespace - empty/whitespace-only CSS is valid
	let trimmedCss = css.trim();
	if (!trimmedCss) {
		return "" as SanitizedCSS;
	}

	// Fix double-encoded JSON strings (legacy data corruption)
	// If the CSS starts and ends with quotes and looks like escaped JSON, try to parse it
	if (trimmedCss.startsWith('"') && trimmedCss.endsWith('"')) {
		try {
			const parsed = JSON.parse(trimmedCss);
			if (typeof parsed === "string") {
				trimmedCss = parsed.trim();
			}
		} catch {
			// Not valid JSON, continue with original
		}
	}

	let root: Root;
	try {
		root = postcss.parse(trimmedCss);
	} catch (error) {
		// If CSS can't be parsed, reject it entirely
		// Extract useful error info for debugging
		const cssError = error as {
			line?: number;
			column?: number;
			reason?: string;
		};
		const location = cssError.line
			? ` at line ${cssError.line}:${cssError.column || 0}`
			: "";
		const reason = cssError.reason || "Parse error";
		console.warn(
			`[safeScopedCss] Invalid CSS${location}: ${reason}. First 200 chars: ${trimmedCss.slice(0, 200)}`,
		);
		return "" as SanitizedCSS;
	}

	// Process top-level rules
	root.walkRules((rule) => {
		// Skip rules inside at-rules (they're handled by processAtRule)
		if (rule.parent?.type === "atrule") {
			return;
		}
		processRule(rule, scopeSelector, false, options);
	});

	// Process at-rules
	root.walkAtRules((atRule) => {
		// Only process top-level at-rules
		if (atRule.parent?.type === "root") {
			processAtRule(atRule, scopeSelector, options);
		}
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
