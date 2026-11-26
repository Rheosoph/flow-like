import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = { locales: { ...(await import("@flow-like/locales")) } };
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { DiscussionProposalCard } = await import("./discussion-proposal");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	restoreDomGlobals();
});

type CardProps = Parameters<typeof DiscussionProposalCard>[0];

async function show(props: Partial<CardProps>) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(DiscussionProposalCard, {
				botName: "Tiro",
				rounds: 12,
				reason: "Three markets to compare with live prices.",
				estimate: { perMessage: 0.02, sample: 4, total: 0.48 },
				limited: false,
				...props,
			}),
		),
	);
}

async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

function allowButton() {
	return [...container.querySelectorAll("button")].find((button) =>
		button.textContent?.includes("Allow 12 rounds"),
	);
}

describe("a bot's proposal for a longer discussion", () => {
	test("shows the rounds, why, the cost and allows it once", async () => {
		let allowed = 0;
		await show({ onAllow: () => allowed++ });
		const text = container.textContent ?? "";
		expect(text).toContain("Tiro suggests a longer discussion");
		expect(text).toContain("Up to 12 rounds, 24 messages");
		expect(text).toContain("Three markets to compare with live prices.");
		expect(text).toContain("About $0.480 in model costs");
		expect(text).toContain("Each bot's owner pays for their own bot.");
		expect(text).not.toContain("your 20 a day");
		expect(text).toContain("The bots wait until someone allows it.");
		const button = allowButton();
		await act(async () => button?.click());
		await act(async () => button?.click());
		expect(allowed).toBe(1);
		expect(button?.hasAttribute("disabled")).toBe(true);
		await hide();
	});

	test("says who allowed it, without a button, and when costs are unknown", async () => {
		await show({
			allowedBy: { name: "Mara", rounds: 12 },
			estimate: { perMessage: null, sample: 0, total: null },
			limited: true,
			onAllow: () => {},
		});
		const text = container.textContent ?? "";
		expect(text).toContain("Mara allowed 12 rounds");
		expect(text).toContain("No reply here has reported its model cost yet.");
		expect(text).toContain("your 20 a day");
		expect(allowButton()).toBeUndefined();
		await hide();

		await show({ allowedBy: { name: null, rounds: 12 } });
		expect(container.textContent).toContain("You allowed 12 rounds");
		await hide();
	});

	test("offers no button to a reader who can't write here", async () => {
		await show({ onAllow: undefined });
		expect(allowButton()).toBeUndefined();
		await hide();
	});
});
