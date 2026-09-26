import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { SOURCE_RESOURCES } from "@flow-like/locales";
import {
	auditActionKey,
	auditActorKind,
	auditDetailEntries,
	humanizeAuditAction,
	parseAuditActor,
} from "./audit-sentence";
import { parseSavedHead } from "./saved-head";

const LEVEL_RS = join(import.meta.dir, "../../../api/src/audit/level.rs");

/** Built with `format!` in `audit/execution.rs` and `execution/rejection.rs`. */
const EXECUTION_ACTIONS = ["board", "event"].flatMap((kind) =>
	["start", "complete", "fail", "cancel", "timeout", "reject"].map(
		(outcome) => `execution.${kind}.${outcome}`,
	),
);

/** Recorded by the app export routes (spec section 5). */
const EXPORT_ACTIONS = [
	"audit.export.webhook.set",
	"audit.export.webhook.rotate",
	"audit.export.webhook.delete",
];

function classifiedActions(): string[] {
	const source = readFileSync(LEVEL_RS, "utf8");
	return [...source.matchAll(/\(\s*"([a-z_.]+)",\s*AuditLevel::/g)].map(
		(match) => match[1],
	);
}

describe("audit action sentences", () => {
	const sentences = SOURCE_RESOURCES.audit.actions as Record<string, string>;
	const recorded = new Set([
		...classifiedActions(),
		...EXECUTION_ACTIONS,
		...EXPORT_ACTIONS,
	]);

	test("the backend classification table was found", () => {
		expect(recorded.size).toBeGreaterThan(EXECUTION_ACTIONS.length + 50);
	});

	test("every recorded action has a sentence", () => {
		const missing = [...recorded].filter(
			(action) => !sentences[auditActionKey(action)]?.trim(),
		);
		expect(missing).toEqual([]);
	});

	test("every sentence belongs to a recorded action", () => {
		const keys = new Set([...recorded].map(auditActionKey));
		expect(Object.keys(sentences).filter((key) => !keys.has(key))).toEqual([]);
	});

	test("an unknown action reads as its humanized name", () => {
		expect(humanizeAuditAction("something.brand_new")).toBe(
			"something brand new",
		);
	});
});

describe("parseAuditActor", () => {
	test("reads the account behind an interactive login", () => {
		expect(parseAuditActor("openid:user-1:", "USER")).toEqual({
			kind: "user",
			method: "openid",
			userId: "user-1",
			reference: undefined,
			raw: "openid:user-1:",
		});
	});

	test("keeps the token id of a personal access token", () => {
		const actor = parseAuditActor("pat:user-1:pat-9", "User");
		expect(actor.userId).toBe("user-1");
		expect(actor.reference).toBe("pat-9");
	});

	test("an app-owned API key has no account", () => {
		const actor = parseAuditActor("api_key:app:app-1:key-1", "API_KEY");
		expect(actor.kind).toBe("apiKey");
		expect(actor.userId).toBeUndefined();
		expect(actor.reference).toBe("key-1");
	});

	test("system actors stay raw", () => {
		expect(parseAuditActor("execution-admission", "SYSTEM")).toEqual({
			kind: "system",
			raw: "execution-admission",
		});
	});

	test("both enum spellings map to the same kind", () => {
		expect(auditActorKind("TECHNICAL_USER")).toBe("technicalUser");
		expect(auditActorKind("TechnicalUser")).toBe("technicalUser");
		expect(auditActorKind("EXECUTOR")).toBe("executor");
	});
});

describe("auditDetailEntries", () => {
	test("flattens ids and codes into strings", () => {
		expect(
			auditDetailEntries({ row_count: 3, user_scoped: false, board: null }),
		).toEqual([
			["row_count", "3"],
			["user_scoped", "false"],
			["board", "null"],
		]);
	});

	test("nothing to show for missing details", () => {
		expect(auditDetailEntries(null)).toEqual([]);
		expect(auditDetailEntries(undefined)).toEqual([]);
	});
});

describe("parseSavedHead", () => {
	const hash = "ab".repeat(32);

	test("reads an AuditHead and a daily head file", () => {
		expect(
			parseSavedHead(
				JSON.stringify({ chain_id: "x", seal: null, epoch: { seq: 4, hash } }),
			),
		).toEqual({ seq: 4, hash });
		expect(
			parseSavedHead(
				JSON.stringify({ epoch: { seq: 7, hash }, written_at_ms: 1 }),
			),
		).toEqual({ seq: 7, hash });
	});

	test("sends the seal of an AuditHead along with its epoch", () => {
		const sealHash = "cd".repeat(32);
		expect(
			parseSavedHead(
				JSON.stringify({
					chain_id: "app-1",
					seal: { chain_id: "app-1", seq: 12, hash: sealHash.toUpperCase() },
					epoch: { seq: 4, hash },
				}),
			),
		).toEqual({
			seq: 4,
			hash,
			chain_id: "app-1",
			seal_seq: 12,
			seal_hash: sealHash,
		});
		expect(
			parseSavedHead(
				JSON.stringify({
					seal: { chain_id: "app-2", seq: 1, hash: sealHash },
					epoch: { seq: 4, hash },
				}),
			),
		).toMatchObject({ chain_id: "app-2", seal_seq: 1 });
	});

	test("a head with a malformed seal is rejected", () => {
		expect(
			parseSavedHead(
				JSON.stringify({
					chain_id: "app-1",
					seal: { seq: 12, hash: "abc" },
					epoch: { seq: 4, hash },
				}),
			),
		).toBeNull();
		expect(
			parseSavedHead(
				JSON.stringify({ seal: { seq: 12, hash }, epoch: { seq: 4, hash } }),
			),
		).toBeNull();
	});

	test("reads a bare epoch line and normalizes the hash", () => {
		expect(
			parseSavedHead(JSON.stringify({ seq: 2, hash: hash.toUpperCase() })),
		).toEqual({ seq: 2, hash });
	});

	test("rejects anything without a sequence and a 32-byte hex hash", () => {
		expect(parseSavedHead("not json")).toBeNull();
		expect(parseSavedHead(JSON.stringify({ epoch: null }))).toBeNull();
		expect(parseSavedHead(JSON.stringify({ seq: 1, hash: "abc" }))).toBeNull();
		expect(parseSavedHead(JSON.stringify({ seq: 1.5, hash }))).toBeNull();
	});
});
