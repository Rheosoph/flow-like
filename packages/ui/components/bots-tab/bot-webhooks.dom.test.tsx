import { afterAll, beforeAll, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IIssuedBotWebhook } from "../../state/backend-state/bot-state";
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
const { IssuedCredentials, WebhookExamples } = await import("./bot-webhooks");
const url = "https://api.example.com/api/v1/bots/hooks/bh_example";
const issued: IIssuedBotWebhook & { url: string } = {
	url,
	secret: "whsec_example",
	signing_key: "example_signing_key",
	webhook: {
		id: "bh_example",
		bot_id: "bot_example",
		name: "New order",
		instruction: "Summarize the order",
		enabled: true,
		path: "/bots/hooks/bh_example",
		created_at: "2026-11-04T12:00:00Z",
	},
};

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	restoreDomGlobals();
});

test("issued credentials offer a secret URL and copyable unsigned curl requests", async () => {
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as Element);
	const done = mock(() => {});
	await act(async () =>
		root.render(createElement(IssuedCredentials, { issued, onDone: done })),
	);
	try {
		expect(
			(
				container.querySelector(
					'input[aria-label="URL with secret"]',
				) as unknown as HTMLInputElement | null
			)?.value,
		).toBe(`${url}?secret=whsec_example`);
		expect(container.textContent).toContain("HMAC signing is optional");
		expect(container.querySelector("details")?.open).toBe(false);
		const examples = container.querySelectorAll("pre code");
		expect(examples).toHaveLength(2);
		expect(examples[0].textContent).toContain(
			"X-Flow-Like-Webhook-Secret: whsec_example",
		);
		expect(examples[1].textContent).toContain("?secret=whsec_example");
		expect(examples[1].textContent).not.toContain("X-Flow-Like-Signature");
		await act(async () =>
			(
				container.querySelector(
					'button[aria-label="Copy Secret in the URL"]',
				) as unknown as HTMLButtonElement | null
			)?.click(),
		);
		expect(await window.navigator.clipboard.readText()).toBe(
			examples[1].textContent,
		);
		await act(async () =>
			Array.from(container.querySelectorAll("button"))
				.find((button) => button.textContent === "I saved it")
				?.click(),
		);
		expect(done).toHaveBeenCalledTimes(1);
	} finally {
		await act(async () => root.unmount());
		container.remove();
	}
});

test("existing webhook examples explain how to supply the saved secret", async () => {
	const container = window.document.createElement("div");
	const root = createRoot(container as unknown as Element);
	await act(async () => root.render(createElement(WebhookExamples, { url })));
	try {
		expect(container.textContent).toContain(
			"Replace YOUR_WEBHOOK_SECRET with your saved secret",
		);
		expect(container.querySelector("pre")?.textContent).toContain(
			"X-Flow-Like-Webhook-Secret: YOUR_WEBHOOK_SECRET",
		);
	} finally {
		await act(async () => root.unmount());
	}
});
