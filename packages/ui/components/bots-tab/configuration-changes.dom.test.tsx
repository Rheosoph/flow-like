import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
const { createRoot } = await import("react-dom/client");
const { BotConfigurationChanges } = await import("./configuration-changes");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => {
	installDomGlobals();
	mock.module("@flow-like/locales", () => ({
		...actualLocales,
		useTranslation: () => ({ t: translate, i18n: fakeI18n }),
	}));
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

beforeEach(async () => {
	await act(async () => root.render(null));
});

afterAll(async () => {
	await act(async () => root.unmount());
	container.remove();
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

async function show(changes: Record<string, unknown>) {
	await act(async () => {
		root.render(createElement(BotConfigurationChanges, { changes }));
	});
}

function readableProposal() {
	const content = container.cloneNode(true) as HTMLElement;
	for (const details of content.querySelectorAll("details")) details.remove();
	return content;
}

function button(label: string) {
	return [...container.querySelectorAll("button")].find(
		(candidate) => candidate.textContent?.trim() === label,
	);
}

describe("BotConfigurationChanges", () => {
	test("names the full capability list and explains replacement", async () => {
		await show({ capabilities: ["internet", "memory"] });
		const text = readableProposal().textContent ?? "";
		expect(text).toContain("Internet");
		expect(text).toContain("Memory");
		expect(text).toContain("Replaces the current capability list.");
		expect(text).not.toContain('["internet"');
	});

	test("shows model roles and preserves fallback priority and complete IDs", async () => {
		const fallbacks = ["profile:chat-fallback-2", "profile:chat-fallback-1"];
		await show({
			model: {
				primary: "profile:chat-primary",
				decision: "profile:chat-decision",
				fallbacks,
			},
		});
		const proposal = readableProposal();
		const text = proposal.textContent ?? "";
		for (const label of ["Primary model", "Decision model", "Fallback models"])
			expect(text).toContain(label);
		expect(text).toContain("profile:chat-primary");
		expect(text).toContain("profile:chat-decision");
		const ordered = proposal.querySelector("ol");
		expect(ordered).not.toBeNull();
		const items = [...(ordered?.querySelectorAll("li") ?? [])];
		expect(items).toHaveLength(2);
		expect(items[0].textContent).toContain(fallbacks[0]);
		expect(items[1].textContent).toContain(fallbacks[1]);
	});

	test("does not imply changes to omitted model roles", async () => {
		await show({ model: { primary: "profile:chat-primary" } });
		const text = readableProposal().textContent ?? "";
		expect(text).toContain("Primary model");
		expect(text).not.toContain("Decision model");
		expect(text).not.toContain("Fallback models");
	});

	test("distinguishes removed assignments, disabled decisions and cleared lists", async () => {
		await show({
			image_model: null,
			model: { decision: null, fallbacks: [] },
			capabilities: [],
		});
		const text = readableProposal().textContent ?? "";
		expect(text).toContain("Remove model assignment");
		expect(text).toContain("Disable decision model");
		expect(text).toContain("Remove all fallback models");
		expect(text).toContain("Remove all capabilities");
	});

	test("expands long instructions without dropping any proposed text", async () => {
		const instructions = `${"Check this complete instruction.\n".repeat(100)}Final instruction.`;
		await show({ instructions });
		const expand = button("Show full text");
		expect(expand).toBeDefined();
		expect(expand?.getAttribute("aria-expanded")).toBe("false");
		const textId = expand?.getAttribute("aria-controls");
		expect(textId).toBeTruthy();
		const fullText = document.getElementById(textId ?? "");
		expect(fullText?.textContent).toBe(instructions);

		await act(async () => expand?.click());
		const collapse = button("Show less");
		expect(collapse).toBeDefined();
		expect(collapse?.getAttribute("aria-expanded")).toBe("true");
		expect(collapse?.getAttribute("aria-controls")).toBe(textId);
		expect(fullText?.textContent).toBe(instructions);

		await act(async () => collapse?.click());
		expect(button("Show full text")?.getAttribute("aria-expanded")).toBe(
			"false",
		);
		expect(fullText?.textContent).toBe(instructions);
	});

	test("keeps exact payload available in collapsed technical details", async () => {
		const changes = {
			name: "Atlas",
			instructions: "Keep line one.\nKeep line two.",
			model: { decision: null, fallbacks: ["profile:backup"] },
		};
		await show(changes);
		const details = container.querySelector("details");
		expect(details).not.toBeNull();
		expect(details?.open).toBe(false);
		expect(details?.querySelector("summary")?.textContent).toContain(
			"Technical details",
		);
		expect(
			JSON.parse(details?.querySelector("pre")?.textContent ?? "null"),
		).toEqual(changes);
	});

	test("preserves nested values from unfamiliar fields outside technical details", async () => {
		await show({
			future_setting: {
				region: "eu-central-1",
				limits: { per_hour: 17 },
				allowed: ["scope:read", "scope:write"],
			},
		});
		const text = readableProposal().textContent ?? "";
		for (const value of ["eu-central-1", "17", "scope:read", "scope:write"])
			expect(text).toContain(value);
		expect(text).not.toContain('"region":');
		expect(text).not.toContain("[object Object]");
	});

	test("identifies an empty proposal", async () => {
		await show({});
		expect(readableProposal().textContent).toContain("No settings to change");
	});
});
