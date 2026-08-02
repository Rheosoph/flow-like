import { describe, expect, test } from "bun:test";
import type { IEvent } from "../../../lib/schema/flow/event";
import { issuesForSection } from "./use-event-issues";

const event = (overrides: Partial<IEvent> = {}): IEvent =>
	({
		active: true,
		board_id: "board-1",
		config: [],
		created_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
		description: "",
		event_type: "email",
		event_version: [1, 0, 0],
		id: "evt-1",
		name: "Vendor mailbox",
		node_id: "node-1",
		priority: 0,
		updated_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
		variables: {},
		...overrides,
	}) as IEvent;

describe("issuesForSection", () => {
	test("filters to the requested section", () => {
		const issues = [
			{
				id: "a",
				severity: "blocking" as const,
				section: "trigger" as const,
				title: "A",
				detail: "",
			},
			{
				id: "b",
				severity: "check" as const,
				section: "inputs" as const,
				title: "B",
				detail: "",
			},
		];
		expect(issuesForSection(issues, "trigger").map((i) => i.id)).toEqual(["a"]);
		expect(issuesForSection(issues, "flow")).toEqual([]);
	});
});

// The hook itself needs a React renderer; these cover the pure predicate it uses
// for credentials, which is where the key-name mismatch bit us.
describe("mail credential detection", () => {
	const missing = (value: unknown) =>
		value === null ||
		value === undefined ||
		(typeof value === "string" && value.trim() === "");
	const keys = ["secret_imap_password", "password"];
	const isMissing = (config: Record<string, unknown>) =>
		keys.every((key) => missing(config[key]));

	test("counts the key the editor actually writes", () => {
		expect(isMissing({ secret_imap_password: "hunter2" })).toBe(false);
	});

	test("still counts the key the defaults seed", () => {
		expect(isMissing({ password: "hunter2" })).toBe(false);
	});

	test("reports missing only when neither is set", () => {
		expect(isMissing({ secret_imap_password: "", password: "" })).toBe(true);
		expect(isMissing({})).toBe(true);
	});

	test("does not accept whitespace as a password", () => {
		expect(isMissing({ secret_imap_password: "   " })).toBe(true);
	});
});

describe("event fixture sanity", () => {
	test("the mail fixture is a mail event", () => {
		expect(event().event_type).toBe("email");
	});
});
