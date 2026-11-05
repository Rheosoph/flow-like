import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };

beforeAll(() => {
	installDomGlobals();
	mock.module("@flow-like/locales", () => ({
		...actualLocales,
		useTranslation: () => ({ t: translate, i18n: fakeI18n }),
	}));
});

afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

const names: Record<string, string> = {
	momo: "Momo",
	scout: "Scout",
	joerg: "Joerg",
	anna: "Anna",
	paul: "Paul",
};

const directory = (id: string) => ({
	id,
	name: names[id] ?? id,
	isMe: false,
	isBot: id === "momo" || id === "scout",
	isPending: false,
	isResolved: Boolean(names[id]),
});

const said = (el: HTMLElement | null | undefined) =>
	el?.querySelector(".truncate")?.textContent ?? "";

async function line(
	bots: { botId: string; status: "queued" | "running" | "waiting" }[],
	typing: string[],
) {
	const { createRoot } = await import("react-dom/client");
	const { ConversationActivityLine } = await import(
		"./conversation-activity-line"
	);
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const client = new QueryClient({
		defaultOptions: { queries: { enabled: false, retry: false } },
	});
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(ConversationActivityLine, { bots, typing, directory }),
			),
		);
	});
	await settle();
	return container.querySelector<HTMLElement>(
		"[data-fl-conversation-activity]",
	);
}

describe("ConversationActivityLine", () => {
	test("says what each bot status means", async () => {
		expect(said(await line([{ botId: "momo", status: "running" }], []))).toBe(
			"Momo is answering",
		);
		expect(said(await line([{ botId: "momo", status: "queued" }], []))).toBe(
			"Waiting for Momo",
		);
		expect(said(await line([{ botId: "momo", status: "waiting" }], []))).toBe(
			"Momo is waiting for approval",
		);
	});

	test("names one or two typists and folds more", async () => {
		expect(said(await line([], ["joerg"]))).toBe("Joerg is typing");
		expect(said(await line([], ["joerg", "anna"]))).toBe(
			"Joerg and Anna are typing",
		);
		expect(said(await line([], ["joerg", "anna", "paul"]))).toBe(
			"Several people are typing",
		);
	});

	test("lists bots first, joined with a dot", async () => {
		const el = await line(
			[
				{ botId: "momo", status: "running" },
				{ botId: "scout", status: "queued" },
			],
			["joerg"],
		);
		expect(said(el)).toBe(
			"Momo is answering · Waiting for Scout · Joerg is typing",
		);
	});

	test("stays mounted with its height when nothing is going on", async () => {
		const el = await line([], []);
		expect(el).not.toBeNull();
		expect(said(el)).toBe("");
		expect(el?.className).toContain("h-6");
		expect(el?.getAttribute("aria-live")).toBe("polite");
	});

	test("animates the dots only when motion is allowed", async () => {
		const el = await line([{ botId: "momo", status: "running" }], []);
		const dot = el?.querySelector("[aria-hidden] > span");
		expect(dot?.className).toContain("motion-safe:animate-bounce");
	});
});
