import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { IBitTypes } from "../../lib/schema/bit/bit";
import { BitParametersEditor } from "./bit-parameters-editor";

function renderEditor({
	scope = "admin",
	provider = "Hosted",
	bitType = IBitTypes.Llm,
	pricing,
}: {
	scope?: "custom" | "admin";
	provider?: string;
	bitType?: IBitTypes;
	pricing?: unknown;
} = {}) {
	return renderToStaticMarkup(
		<BitParametersEditor
			scope={scope}
			bitType={bitType}
			value={{
				context_length: 4096,
				provider: { provider_name: provider },
				pricing,
			}}
			onChange={() => {}}
			jsonText={null}
			onJsonChange={() => {}}
			jsonError={null}
			onApplyJson={() => {}}
			onResetJson={() => {}}
		/>,
	);
}

test("unpriced hosted models warn admins without inventing rates", () => {
	for (const pricing of [undefined, null]) {
		const markup = renderEditor({ pricing });
		expect(markup).toContain("No price is configured.");
		expect(markup).toContain("Requests still run");
		expect(markup).toContain("Add pricing");
		expect(markup).not.toContain("USD per 1M input tokens");
	}
});

test("hosted pricing fields display exact USD amounts and permit removal", () => {
	const markup = renderEditor({
		bitType: IBitTypes.Vlm,
		provider: "hosted:openrouter",
		pricing: {
			input_micro_usd_per_million_tokens: 249,
			output_micro_usd_per_million_tokens: Number.MAX_SAFE_INTEGER,
		},
	});
	expect(markup).toContain("USD per 1M input tokens");
	expect(markup).toContain('value="0.000249"');
	expect(markup).toContain('value="9007199254.740991"');
	expect(markup).toContain("USD per request");
	expect(markup).toContain("(optional)");
	expect(markup).toContain("Remove pricing");
	expect(markup).not.toContain("No price is configured.");
	expect(markup).not.toContain("Input micro usd per million tokens");
});

test("pricing controls are limited to admin hosted language and vision models", () => {
	for (const props of [
		{ scope: "custom" as const },
		{ provider: "Local" },
		{ provider: "custom:openrouter" },
		{ bitType: IBitTypes.Embedding },
	]) {
		expect(renderEditor(props)).not.toContain("Hosted model pricing");
	}
});

test("admins can add, enter decimal rates, and remove pricing without losing parameters", async () => {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		HTMLElement: window.HTMLElement,
		HTMLInputElement: window.HTMLInputElement,
		Element: window.Element,
		Node: window.Node,
		Event: window.Event,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as Element);
	let parameters: Record<string, unknown> = {
		context_length: 4096,
		provider: { provider_name: "Hosted" },
		unknown: { retained: true },
	};
	function Harness() {
		const [value, setValue] = useState(parameters);
		return (
			<BitParametersEditor
				scope="admin"
				bitType={IBitTypes.Llm}
				value={value}
				onChange={(next) => {
					parameters = next as Record<string, unknown>;
					setValue(parameters);
				}}
				jsonText={null}
				onJsonChange={() => {}}
				jsonError={null}
				onApplyJson={() => {}}
				onResetJson={() => {}}
			/>
		);
	}
	function button(text: string) {
		const found = [...container.querySelectorAll("button")].find(
			(item) => item.textContent === text,
		);
		if (!found) throw new Error(`Missing button: ${text}`);
		return found;
	}
	try {
		await act(async () => root.render(<Harness />));
		await act(async () => button("Add pricing").click());
		expect(parameters.pricing).toEqual({});
		const label = [...container.querySelectorAll("label")].find((item) =>
			item.textContent?.startsWith("USD per 1M input tokens"),
		);
		const input = window.document.getElementById(
			label?.getAttribute("for") ?? "",
		) as unknown as HTMLInputElement;
		await act(async () => input.focus());
		const setValue = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		for (const text of ["0", "0.", "0.000249"]) {
			await act(async () => {
				setValue?.call(input, text);
				input.dispatchEvent(
					new window.Event("input", { bubbles: true }) as unknown as Event,
				);
			});
			expect(input.value).toBe(text);
		}
		expect(parameters.pricing).toEqual({
			input_micro_usd_per_million_tokens: 249,
		});
		await act(async () => input.blur());
		expect(input.value).toBe("0.000249");
		await act(async () => button("Remove pricing").click());
		expect(parameters.pricing).toBeUndefined();
		expect(parameters.unknown).toEqual({ retained: true });
		expect(container.textContent).toContain("Requests still run");
	} finally {
		await act(async () => root.unmount());
		for (const key of Object.keys(globals)) {
			const descriptor = previous[key];
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
		window.happyDOM.abort();
	}
});
