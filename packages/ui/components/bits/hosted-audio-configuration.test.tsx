import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";
import { validateHostedAudioParameters } from "../../lib/bit/hosted-audio";
import { HostedAudioConfiguration } from "./hosted-audio-configuration";

test("changing audio providers clears the old model while retaining the access tier", async () => {
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
			provider_name: "hosted:openrouter",
			model_id: "provider/speech",
			params: { tier: "FREE" },
		},
	};
	function Harness() {
		const [value, setValue] = useState(parameters);
		return (
			<HostedAudioConfiguration
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
		await input("Default voice", "af_heart");
		await input("Default language", "pt-BR");
		expect(parameters.default_voice).toBe("af_heart");
		expect(parameters.default_language).toBe("pt-BR");
		await input("Model ID", "hexgrad/kokoro-82m");
		expect(parameters.default_voice).toBe("af_heart");
		await select("Provider", "hosted:groq");
		expect(parameters).toEqual({
			provider: {
				provider_name: "hosted:groq",
				model_id: "",
				params: { tier: "FREE" },
			},
		});
		expect(container.textContent).toContain("HOSTED_GROQ_API_KEY");
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"model ID",
		);
		await input("Model ID", "whisper-large-v3");
		expect(validateHostedAudioParameters(parameters)).toBeNull();
		await input("Default voice", "tara");
		await input("Default voice", "");
		expect(parameters).not.toHaveProperty("default_voice");
		await input("Access tier", "");
		expect(parameters).toEqual({
			provider: {
				provider_name: "hosted:groq",
				model_id: "whisper-large-v3",
				params: {},
			},
		});
	} finally {
		await act(async () => root.unmount());
		window.happyDOM.abort();
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	}
});
