// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { exchangesOfBot, parseAskBotResult } from "./bot-exchange";
import { BotExchangeCard, BotExchangeList } from "./bot-exchange-card";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown, options?: Record<string, unknown>) =>
			(typeof fallback === "string" ? fallback : key).replace(
				/\{\{(\w+)\}\}/g,
				(_, name: string) => String(options?.[name] ?? ""),
			),
	}),
}));

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

const result = (status: string, extra: Record<string, unknown> = {}) =>
	JSON.stringify({
		status: status === "answered" ? "ok" : "error",
		tool: "ask_bot",
		message: "Ledger is offline; the question waits in its inbox.",
		exchange: {
			conversation_id: "conv-1",
			callee: { id: "bot_ledger", name: "Ledger" },
			question: "What is the total?",
			answer: status === "answered" ? "42" : null,
			status,
			...extra,
		},
	});

describe("parseAskBotResult", () => {
	test("reads an answered exchange", () => {
		const view = parseAskBotResult(result("answered"), "Scout");
		expect(view).toMatchObject({
			state: "answered",
			calleeName: "Ledger",
			answer: "42",
			conversationId: "conv-1",
			note: null,
		});
	});

	test("maps pending, parked and denied; anything else is an error", () => {
		expect(parseAskBotResult(result("pending"), "Scout")?.state).toBe("pending");
		expect(parseAskBotResult(result("parked"), "Scout")?.state).toBe("waiting");
		expect(parseAskBotResult(result("denied"), "Scout")?.state).toBe("denied");
		expect(parseAskBotResult(result("timeout"), "Scout")?.state).toBe("error");
	});

	test("ignores other tools and unreadable text", () => {
		expect(parseAskBotResult("not json", "Scout")).toBeNull();
		expect(
			parseAskBotResult({ tool: "list_apps", exchange: {} }, "Scout"),
		).toBeNull();
	});
});

describe("exchangesOfBot", () => {
	const members = (...bots: string[]) => [
		{ principal_id: "user-a", kind: "user" },
		...bots.map((id) => ({ principal_id: id, kind: "bot" })),
	];
	test("lists the groups of two bots that include this one, newest first", () => {
		const conversations = [
			{ id: "old", kind: "group", title: null, last_activity_at: "2026-11-01", members: members("a", "b") },
			{ id: "new", kind: "group", title: "A <-> C", last_activity_at: "2026-11-03", members: members("a", "c") },
			{ id: "other", kind: "group", title: null, last_activity_at: "2026-11-03", members: members("b", "c") },
			{ id: "direct", kind: "direct", title: null, last_activity_at: "2026-11-03", members: members("a") },
			{ id: "crowd", kind: "group", title: null, last_activity_at: "2026-11-03", members: members("a", "b", "c") },
		];
		expect(exchangesOfBot(conversations, "a").map((entry) => entry.id)).toEqual([
			"new",
			"old",
		]);
	});
});

describe("BotExchangeCard", () => {
	test("shows who asked whom, the answer and a link to the full exchange", async () => {
		const view = parseAskBotResult(result("answered"), "Scout");
		const onOpen = vi.fn();
		await act(async () =>
			root.render(<BotExchangeCard exchange={view!} onOpen={onOpen} />),
		);
		expect(host.textContent).toContain("Scout asked Ledger");
		expect(host.textContent).toContain("What is the total?");
		expect(host.textContent).toContain("Ledger answered");
		expect(host.textContent).toContain("42");
		const link = host.querySelector("button");
		await act(async () => link?.click());
		expect(onOpen).toHaveBeenCalledWith("conv-1");
	});

	test("an offline callee reads as waiting with the reason", async () => {
		const view = parseAskBotResult(result("pending"), "Scout");
		await act(async () => root.render(<BotExchangeCard exchange={view!} />));
		const card = host.querySelector("section");
		expect(card?.getAttribute("data-state")).toBe("pending");
		expect(host.textContent).toContain("waiting for the answer");
		expect(host.textContent).toContain("Ledger is offline");
		expect(host.querySelector("button")).toBeNull();
	});

	test("a parked ask says it needs permission", async () => {
		const view = parseAskBotResult(result("parked"), "Scout");
		await act(async () => root.render(<BotExchangeCard exchange={view!} />));
		expect(host.textContent).toContain("needs your permission");
	});
});

describe("BotExchangeList", () => {
	test("lists exchanges read-only and opens one", async () => {
		const onOpen = vi.fn();
		await act(async () =>
			root.render(
				<BotExchangeList
					entries={[{ id: "c1", title: "t", otherBotId: "b", lastActivityAt: "x" }]}
					nameOf={() => "Ledger"}
					onOpen={onOpen}
				/>,
			),
		);
		expect(host.textContent).toContain("With Ledger");
		expect(host.textContent).toContain("Read only");
		await act(async () => host.querySelector("button")?.click());
		expect(onOpen).toHaveBeenCalledWith("c1");
	});

	test("says so when there are none", async () => {
		await act(async () =>
			root.render(<BotExchangeList entries={[]} nameOf={String} onOpen={vi.fn()} />),
		);
		expect(host.textContent).toContain("has not talked");
	});
});
