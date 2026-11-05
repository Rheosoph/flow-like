import { afterAll, beforeAll, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IBotSkill } from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
const actualHooks = { ...(await import("../../hooks/use-bots")) };

const skills: IBotSkill[] = [
	{
		id: "s-active",
		bot_id: "b1",
		name: "pdf-forms",
		description: "Fill PDF forms",
		body: "1. Do it",
		source: "owner",
		status: "active",
		tools: [],
	},
	{
		id: "s-pending",
		bot_id: "b1",
		name: "weekly-report",
		description: "Write the report",
		body: "1. Collect",
		source: "bot_proposed",
		status: "pending",
		tools: [],
	},
];
const actions: unknown[] = [];

mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));
mock.module("../../hooks/use-bots", () => ({
	...actualHooks,
	useBotSkills: () => ({ data: skills, isLoading: false }),
	useBotSkillAction: () => ({
		isPending: false,
		mutate: (variables: unknown) => actions.push(variables),
	}),
}));

const { createRoot } = await import("react-dom/client");
const { BotSkillsCard } = await import("./bot-skills");

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../hooks/use-bots", () => actualHooks);
	restoreDomGlobals();
});

test("a pending skill waits for approval and approving sends the action", async () => {
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as Element);
	await act(async () =>
		root.render(createElement(BotSkillsCard, { botId: "b1" })),
	);
	try {
		const rows = [...container.querySelectorAll('[data-slot="bot-skill"]')];
		expect(rows.map((row) => row.getAttribute("data-status"))).toEqual([
			"pending",
			"active",
		]);
		expect(rows[0]?.textContent).toContain(
			"can't see this skill until you approve",
		);
		const approve = [...rows[0].querySelectorAll("button")].find((button) =>
			button.textContent?.includes("Approve"),
		);
		await act(async () => {
			approve?.dispatchEvent(
				new window.MouseEvent("click", { bubbles: true }) as unknown as Event,
			);
		});
		expect(actions).toEqual([
			{ botId: "b1", action: { type: "approve", skillId: "s-pending" } },
		]);
		expect(rows[1]?.textContent).not.toContain("until you approve");
	} finally {
		await act(async () => root.unmount());
		container.remove();
	}
});
