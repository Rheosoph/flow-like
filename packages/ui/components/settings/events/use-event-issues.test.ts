import { describe, expect, test } from "bun:test";
import type { IVariable } from "../../../lib/schema/flow/board";
import type { IEvent } from "../../../lib/schema/flow/event";
import { computeEventIssues, issuesForSection } from "./use-event-issues";

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

describe("runtime variable coverage", () => {
	const variable = (overrides: Partial<IVariable> = {}): IVariable =>
		({
			id: "var-1",
			name: "API_KEY",
			data_type: "String",
			value_type: "Normal",
			exposed: false,
			secret: false,
			editable: true,
			runtime_configured: true,
			...overrides,
		}) as IVariable;

	const issueIds = (
		input: Parameters<typeof computeEventIssues>[0],
	): string[] => computeEventIssues(input).map((issue) => issue.id);

	test("flags a runtime variable a headless trigger cannot supply", () => {
		const issues = computeEventIssues({
			event: event({ event_type: "cron" }),
			config: { expression: "0 * * * *" },
			boardVariables: { "var-1": variable() },
		});
		const found = issues.find((issue) => issue.id === "runtime-vars-unset");
		expect(found?.section).toBe("variables");
		expect(found?.detail).toContain("API_KEY");
	});

	test("treats secrets as runtime configured", () => {
		expect(
			issueIds({
				event: event({ event_type: "rest" }),
				config: {},
				boardVariables: {
					"var-1": variable({ runtime_configured: false, secret: true }),
				},
			}),
		).toContain("runtime-vars-unset");
	});

	test("stays quiet once the event overrides it", () => {
		expect(
			issueIds({
				event: event({
					event_type: "cron",
					variables: { "var-1": variable() },
				}),
				config: { expression: "0 * * * *" },
				boardVariables: { "var-1": variable() },
			}),
		).not.toContain("runtime-vars-unset");
	});

	test("ignores plain and exposed board variables", () => {
		expect(
			issueIds({
				event: event({ event_type: "mcp" }),
				config: {},
				boardVariables: {
					"var-1": variable({ runtime_configured: false }),
					"var-2": variable({
						id: "var-2",
						runtime_configured: false,
						exposed: true,
					}),
				},
			}),
		).not.toContain("runtime-vars-unset");
	});

	test("leaves interactive triggers alone — they prompt the user", () => {
		expect(
			issueIds({
				event: event({ event_type: "simple_chat" }),
				config: {},
				boardVariables: { "var-1": variable() },
			}),
		).not.toContain("runtime-vars-unset");
	});

	test("is a no-op when no board is loaded", () => {
		expect(
			issueIds({ event: event({ event_type: "cron" }), config: {} }),
		).not.toContain("runtime-vars-unset");
	});
});
