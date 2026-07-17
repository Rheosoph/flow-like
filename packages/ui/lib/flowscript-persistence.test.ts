import { describe, expect, test } from "bun:test";
import {
	sanitizeFlowScriptForPersistence,
	sanitizePotentialFlowScriptTextForPersistence,
} from "./flowscript-persistence";

const SUPPORTED_SECRET_SOURCE = `@secret
const imap_password: string = "mailbox-secret"

eventsSimple() {
  logInfo({ message: "safe" })
}`;

const UNSUPPORTED_SECRET_SOURCE =
	'prefix @secret const innocuous: string = "must-not-leak"';

describe("sanitizePotentialFlowScriptTextForPersistence", () => {
	test("sanitizes only source-like fields of a JSON tool payload", () => {
		const payload = JSON.stringify({
			status: "validation_errors",
			revision: 16,
			source: UNSUPPORTED_SECRET_SOURCE,
			structured_diagnostics: [
				{
					code: "FS_FUNCTION_RETURN_MISMATCH",
					phase: "reconcile",
					message: "return value 1 is not a resolvable FlowScript value",
				},
			],
			review_notes: ["The draft was blocked before apply."],
			message: "Nothing was queued.",
		});

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);
		const parsed = JSON.parse(sanitized) as Record<string, unknown>;

		expect(parsed.status).toBe("validation_errors");
		expect(parsed.revision).toBe(16);
		expect(parsed.structured_diagnostics).toEqual([
			{
				code: "FS_FUNCTION_RETURN_MISMATCH",
				phase: "reconcile",
				message: "return value 1 is not a resolvable FlowScript value",
			},
		]);
		expect(parsed.review_notes).toEqual([
			"The draft was blocked before apply.",
		]);
		expect(parsed.message).toBe("Nothing was queued.");
		expect(String(parsed.source)).not.toContain("must-not-leak");
		expect(String(parsed.source)).toContain(
			"Persisted FlowScript copy omitted",
		);
	});

	test("redacts supported secret declarations inside a JSON payload in place", () => {
		const payload = JSON.stringify({
			status: "queued",
			source: SUPPORTED_SECRET_SOURCE,
		});

		const parsed = JSON.parse(
			sanitizePotentialFlowScriptTextForPersistence(payload),
		) as Record<string, unknown>;

		expect(String(parsed.source)).toContain('const imap_password: string = ""');
		expect(String(parsed.source)).not.toContain("mailbox-secret");
		expect(parsed.status).toBe("queued");
	});

	test("descends into nested JSON strings such as MCP text envelopes", () => {
		const payload = JSON.stringify([
			{
				type: "text",
				text: JSON.stringify({
					status: "validation_errors",
					source: UNSUPPORTED_SECRET_SOURCE,
					structured_diagnostics: [{ code: "FS_PARSE_ERROR" }],
				}),
			},
		]);

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);

		expect(sanitized).not.toContain("must-not-leak");
		expect(sanitized).toContain("FS_PARSE_ERROR");
		expect(sanitized).toContain("validation_errors");
	});

	test("fail-closed redacts non-source prose carrying the annotation marker", () => {
		// Key-independent by design: a field name is never an exemption, so even prose that only
		// mentions @secret is redacted rather than risking a missed declaration.
		const payload = JSON.stringify({
			status: "validation_errors",
			diagnostics: ["Unsupported @secret annotation at line 1."],
		});

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);
		const parsed = JSON.parse(sanitized) as {
			status: string;
			diagnostics: string[];
		};

		expect(parsed.status).toBe("validation_errors");
		expect(parsed.diagnostics[0]).toContain(
			"Persisted FlowScript copy omitted",
		);
		expect(sanitized).not.toContain("@secret");
	});

	test("is idempotent for JSON payloads and redaction notices", () => {
		const payload = JSON.stringify({
			status: "validation_errors",
			source: UNSUPPORTED_SECRET_SOURCE,
			diagnostics: ["Unsupported @secret annotation at line 1."],
		});

		const first = sanitizePotentialFlowScriptTextForPersistence(payload);
		const second = sanitizePotentialFlowScriptTextForPersistence(first);
		expect(second).toBe(first);

		const notice = sanitizePotentialFlowScriptTextForPersistence(
			UNSUPPORTED_SECRET_SOURCE,
		);
		expect(sanitizePotentialFlowScriptTextForPersistence(notice)).toBe(notice);

		// A legacy notice still carrying the literal trigger is normalized instead of passed
		// through verbatim: the notice prefix must never exempt trailing @secret content.
		const legacyNotice =
			"[REDACTED] Persisted FlowScript copy omitted for secret safety (not a parser/reconcile error): Unsupported @secret annotation at line 1.";
		const normalized =
			sanitizePotentialFlowScriptTextForPersistence(legacyNotice);
		expect(normalized).toContain("Persisted FlowScript copy omitted");
		expect(normalized).not.toContain("@secret");
		expect(sanitizePotentialFlowScriptTextForPersistence(normalized)).toBe(
			normalized,
		);
	});

	test("the redaction notice never contains the trigger substring", () => {
		const notice = sanitizePotentialFlowScriptTextForPersistence(
			UNSUPPORTED_SECRET_SOURCE,
		);

		expect(notice).toContain("Persisted FlowScript copy omitted");
		expect(notice).toContain("not a parser/reconcile error");
		expect(notice).not.toContain("@secret");
		expect(notice).not.toContain("must-not-leak");
	});

	test("keeps fail-closed behavior for unparseable secret-bearing plain text", () => {
		const sanitized = sanitizePotentialFlowScriptTextForPersistence(
			UNSUPPORTED_SECRET_SOURCE,
		);

		expect(sanitized).toContain("Persisted FlowScript copy omitted");
		expect(sanitized).not.toContain("must-not-leak");

		const malformedJson = `{"source": "@secret const x: string = "broken-quote-secret""`;
		const malformedSanitized =
			sanitizePotentialFlowScriptTextForPersistence(malformedJson);
		expect(malformedSanitized).not.toContain("broken-quote-secret");
		expect(malformedSanitized).toContain("Persisted FlowScript copy omitted");
	});

	test("redacts secret-bearing FlowScript in non-source fields such as patch payloads", () => {
		const payload = JSON.stringify({
			command: "patch",
			old_text: SUPPORTED_SECRET_SOURCE,
			new_text: '@secret\nconst api_key: string = "sk-live-rotated-abcd"',
			message: UNSUPPORTED_SECRET_SOURCE,
			summary: "Rotate the IMAP credentials.",
		});

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);
		const parsed = JSON.parse(sanitized) as Record<string, string>;

		expect(sanitized).not.toContain("mailbox-secret");
		expect(sanitized).not.toContain("sk-live-rotated-abcd");
		expect(sanitized).not.toContain("must-not-leak");
		expect(parsed.old_text).toContain('const imap_password: string = ""');
		expect(parsed.new_text).toContain('const api_key: string = ""');
		expect(parsed.message).toContain("Persisted FlowScript copy omitted");
		expect(parsed.summary).toBe("Rotate the IMAP credentials.");
	});

	test("does not let a prepended redaction notice bypass sanitization", () => {
		const notice = sanitizePotentialFlowScriptTextForPersistence(
			UNSUPPORTED_SECRET_SOURCE,
		);

		const multiline = `${notice}\n@secret\nconst smuggled: string = "sk-live-concat-leak"`;
		const sanitizedMultiline =
			sanitizePotentialFlowScriptTextForPersistence(multiline);
		expect(sanitizedMultiline).not.toContain("sk-live-concat-leak");
		expect(sanitizedMultiline).toContain('const smuggled: string = ""');

		const singleLine = `${notice} @secret const smuggled: string = "sk-live-inline-leak"`;
		const sanitizedSingleLine =
			sanitizePotentialFlowScriptTextForPersistence(singleLine);
		expect(sanitizedSingleLine).not.toContain("sk-live-inline-leak");
		expect(sanitizedSingleLine).toContain("Persisted FlowScript copy omitted");
	});

	test("treats text fields under a source-like parent as source, MCP style", () => {
		const payload = JSON.stringify({
			content: [{ type: "text", text: SUPPORTED_SECRET_SOURCE }],
			status: "ok",
		});

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);
		const parsed = JSON.parse(sanitized) as {
			content: Array<{ type: string; text: string }>;
			status: string;
		};

		expect(sanitized).not.toContain("mailbox-secret");
		expect(parsed.content[0]?.text).toContain(
			'const imap_password: string = ""',
		);
		expect(parsed.status).toBe("ok");
	});

	test("scrubs redacted initializer values echoed into sibling fields", () => {
		const payload = JSON.stringify({
			source: '@secret\nconst api_token: string = "sk-live-echoed-value"',
			diagnostics: [
				{
					code: "FS_TYPE_MISMATCH",
					actual: "sk-live-echoed-value",
					message: 'expected a placeholder, got "sk-live-echoed-value"',
				},
			],
			nested: {
				text: JSON.stringify({ trace: "seen sk-live-echoed-value here" }),
			},
		});

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);
		const parsed = JSON.parse(sanitized) as {
			source: string;
			diagnostics: Array<{ code: string; actual: string; message: string }>;
		};

		expect(sanitized).not.toContain("sk-live-echoed-value");
		expect(parsed.source).toContain('const api_token: string = ""');
		expect(parsed.diagnostics[0]?.code).toBe("FS_TYPE_MISMATCH");
		expect(parsed.diagnostics[0]?.actual).toBe("[REDACTED]");
		expect(parsed.diagnostics[0]?.message).toContain("[REDACTED]");
	});

	test("scrubs echoed initializer values at deeper JSON escape levels", () => {
		const secretValue = 'pa"ss-word-secret';
		const payload = JSON.stringify({
			source: '@secret\nconst t: string = "pa\\"ss-word-secret"',
			nested: {
				content: JSON.stringify({
					type: "text",
					text: JSON.stringify({ trace: `echo ${secretValue} end` }),
				}),
			},
		});

		const sanitized = sanitizePotentialFlowScriptTextForPersistence(payload);
		const parsed = JSON.parse(sanitized) as { source: string };

		expect(sanitized).not.toContain("ss-word-secret");
		expect(parsed.source).toContain('const t: string = ""');
		expect(sanitizePotentialFlowScriptTextForPersistence(sanitized)).toBe(
			sanitized,
		);
	});

	test("every sanitized output is a fixed point of the sanitizer", () => {
		const notice = sanitizePotentialFlowScriptTextForPersistence(
			UNSUPPORTED_SECRET_SOURCE,
		);
		const inputs = [
			SUPPORTED_SECRET_SOURCE,
			UNSUPPORTED_SECRET_SOURCE,
			JSON.stringify({
				command: "patch",
				old_text: SUPPORTED_SECRET_SOURCE,
				message: UNSUPPORTED_SECRET_SOURCE,
			}),
			`${notice}\n@secret\nconst smuggled: string = "sk-live-concat-leak"`,
			JSON.stringify([
				{
					type: "text",
					text: JSON.stringify({ source: UNSUPPORTED_SECRET_SOURCE }),
				},
			]),
			JSON.stringify({
				source: '@secret\nconst api_token: string = "sk-live-echoed-value"',
				diagnostics: [{ actual: "sk-live-echoed-value" }],
			}),
		];

		for (const input of inputs) {
			const once = sanitizePotentialFlowScriptTextForPersistence(input);
			expect(sanitizePotentialFlowScriptTextForPersistence(once)).toBe(once);
		}
	});

	test("passes through supported plain-text FlowScript and secret-free text unchanged", () => {
		expect(
			sanitizePotentialFlowScriptTextForPersistence("no secrets here"),
		).toBe("no secrets here");
		const sanitized = sanitizePotentialFlowScriptTextForPersistence(
			SUPPORTED_SECRET_SOURCE,
		);
		expect(sanitized).toContain('const imap_password: string = ""');
		expect(sanitized).not.toContain("mailbox-secret");
	});
});

describe("sanitizeFlowScriptForPersistence", () => {
	test("still refuses unsupported declarations with a reason", () => {
		const result = sanitizeFlowScriptForPersistence(UNSUPPORTED_SECRET_SOURCE);
		expect(result.safe).toBe(false);
		if (!result.safe) {
			expect(result.reason).toContain("line 1");
		}
	});
});
