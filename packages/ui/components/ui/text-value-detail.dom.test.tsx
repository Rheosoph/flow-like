import { afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

const INVOICE_XML = `<?xml version="1.0" encoding="UTF-8"?>
<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100">
  <ram:ID>RE-2026-0824</ram:ID>
</rsm:CrossIndustryInvoice>`;

const PAYLOAD = JSON.stringify({
	review_state: "needs_review",
	draft_payload_json: JSON.stringify({
		extraction: { currency: "EUR", gross_amount: "1035.30" },
		xml_text: INVOICE_XML,
	}),
});

async function render(text: string) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		window,
		Document: window.Document,
		Text: window.Text,
		MutationObserver: window.MutationObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const { TextValueDetail } = await import("./text-value-detail");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	cleanup = async () => {
		await act(async () => root.unmount());
		await window.happyDOM.close();
	};
	await act(async () => root.render(<TextValueDetail text={text} />));
	return container;
}

const clickButton = async (
	container: { querySelectorAll: Element["querySelectorAll"] },
	label: string,
) => {
	const button = Array.from(container.querySelectorAll("button")).find(
		(candidate) => candidate.textContent === label,
	);
	if (!button) throw new Error(`no ${label} button`);
	await act(async () => (button as unknown as HTMLButtonElement).click());
};

test("JSON serialized into a string field reads as structure, XML inside it verbatim", async () => {
	const container = await render(PAYLOAD);
	const text = container.textContent ?? "";

	expect(text).toContain("JSON object · 2 keys");
	expect(text).toContain("JSON string");
	expect(text).toContain("currency: ");
	expect(text).toContain('"EUR"');
	expect(text).toContain("<ram:ID>RE-2026-0824</ram:ID>");
	expect(text).not.toContain("\\n");
	expect(container.querySelector("a")).toBeNull();
});

test("Raw shows the stored string exactly", async () => {
	const container = await render(PAYLOAD);
	await clickButton(container, "Raw");

	expect(container.querySelector("pre")?.textContent).toBe(PAYLOAD);
});

test("XML has no formatted reading to switch to", async () => {
	const container = await render(INVOICE_XML);

	expect(container.textContent).toContain("Markup");
	expect(container.querySelector("pre")?.textContent).toBe(INVOICE_XML);
	expect(
		Array.from(container.querySelectorAll("button")).some(
			(button) => button.textContent === "Raw",
		),
	).toBe(false);
});
