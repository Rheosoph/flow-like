import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";
import { validateHostedImageParameters } from "../../lib/bit/hosted-image-generation";
import { HostedImageGenerationConfiguration } from "./hosted-image-generation-configuration";

test("changing image providers resets model and price while tier edits preserve other parameters", async () => {
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
		provider: {
			provider_name: "hosted:cloudflare",
			model_id: "@cf/black-forest-labs/flux-1-schnell",
			params: { tier: "FREE" },
		},
		pricing: { request_micro_usd: 2500 },
		unrelated: { retained: true },
	};
	function Harness() {
		const [value, setValue] = useState(parameters);
		return (
			<HostedImageGenerationConfiguration
				parameters={value}
				onChange={(next) => {
					parameters = next;
					setValue(next);
				}}
			/>
		);
	}
	function field(label: string) {
		const match = [...container.querySelectorAll("label")].find((item) =>
			item.textContent?.startsWith(label),
		);
		const element = window.document.getElementById(
			match?.getAttribute("for") ?? "",
		);
		if (!element) throw new Error(`Missing field: ${label}`);
		return element;
	}
	async function select(label: string, value: string) {
		const element = field(label) as unknown as HTMLSelectElement;
		await act(async () => {
			element.value = value;
			element.dispatchEvent(
				new window.Event("change", { bubbles: true }) as unknown as Event,
			);
		});
	}
	async function input(label: string, value: string) {
		const element = field(label) as unknown as HTMLInputElement;
		const setValue = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		await act(async () => {
			setValue?.call(element, value);
			element.dispatchEvent(
				new window.Event("input", { bubbles: true }) as unknown as Event,
			);
		});
	}
	try {
		await act(async () => root.render(<Harness />));
		expect(container.querySelector('[role="alert"]')).toBeNull();
		await select("Provider", "hosted:openrouter");
		expect(parameters).toMatchObject({
			provider: {
				provider_name: "hosted:openrouter",
				model_id: "",
				params: { tier: "FREE" },
			},
			pricing: {},
			unrelated: { retained: true },
		});
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"model ID",
		);
		await input("Model ID", "google/gemini-2.5-flash-image");
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"price",
		);
		await input("Maximum USD per image", "0.1");
		expect(parameters.pricing).toEqual({ request_micro_usd: 100_000 });
		expect(container.querySelector('[role="alert"]')).toBeNull();
		await input("Access tier", "");
		expect(parameters.provider).toMatchObject({ params: {} });
		expect(
			(parameters.provider as { params: Record<string, unknown> }).params,
		).not.toHaveProperty("tier");
		await select("Provider", "hosted:cloudflare");
		expect(parameters.provider).toEqual({
			provider_name: "hosted:cloudflare",
			model_id: "@cf/black-forest-labs/flux-1-schnell",
			params: {},
		});
		expect(parameters.pricing).toEqual({});
		await input("USD per image", "0.002");
		expect(validateHostedImageParameters(parameters)).toBeNull();
		await select("Model ID", "@cf/stabilityai/stable-diffusion-xl-base-1.0");
		expect(parameters.pricing).toEqual({});
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"price",
		);
		expect(parameters.unrelated).toEqual({ retained: true });
	} finally {
		await act(async () => root.unmount());
		window.happyDOM.abort();
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	}
});
