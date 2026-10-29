import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { HostedEmbeddingConfiguration } from "./hosted-embedding-configuration";

function render(
	implementation: string,
	model = "model",
	supportsDimensions?: boolean | null,
) {
	return renderToStaticMarkup(
		<HostedEmbeddingConfiguration
			parameters={{
				remote: {
					implementation,
					model_id: model,
					...(supportsDimensions !== undefined && {
						supports_dimensions: supportsDimensions,
					}),
				},
			}}
			onChange={() => {}}
		/>,
	);
}

test("known Cloudflare models show their batch limits", () => {
	expect(
		render("CloudflareWorkersAI", "@cf/qwen/qwen3-embedding-0.6b"),
	).toContain("32 inputs per request");
	for (const model of [
		"@cf/google/embeddinggemma-300m",
		"@cf/baai/bge-large-en-v1.5",
	]) {
		expect(render("CloudflareWorkersAI", model)).toContain(
			"100 inputs per request",
		);
	}
	expect(render("CloudflareWorkersAI", "another-model")).not.toContain(
		"inputs per request",
	);
});

test("fixed endpoints omit endpoint overrides and external routes show input pricing", () => {
	for (const provider of [
		"CloudflareWorkersAI",
		"OpenAI",
		"OpenRouter",
		"Cohere",
		"VoyageAI",
	]) {
		const markup = render(provider);
		expect(markup).not.toContain("Endpoint secret name");
		expect(markup).toContain("API key secret name");
		expect(markup).toContain("Hosted embedding pricing");
		expect(markup).not.toContain("output tokens");
	}
	for (const provider of [
		"Internal",
		"AzureOpenAI",
		"HuggingfaceEndpoint",
		"OpenAICompatible",
	])
		expect(render(provider)).toContain("Endpoint secret name");
	expect(render("Internal")).not.toContain("Hosted embedding pricing");
});

test("OpenRouter exposes its default key and Gemini multimodal configuration", () => {
	const markup = render("OpenRouter", "google/gemini-embedding-2");
	expect(markup).toContain('placeholder="google/gemini-embedding-2"');
	expect(markup).toContain('placeholder="OPENROUTER_API_KEY"');
	expect(markup).toContain("text, images, audio, video and PDF files");
	expect(markup).toContain("128 to 3072 vector dimensions");
	expect(markup).toContain("at most 8192 tokens");
	expect(markup).toContain("Maximum USD per 1M input tokens");
	expect(markup).toContain("cost reported by OpenRouter");
	expect(markup).not.toContain("Input price unit");
	expect(markup).not.toContain("Input bytes");
	expect(render("OpenRouter", "another-model")).not.toContain(
		"Gemini Embedding 2 accepts",
	);
});

test("OpenRouter dimension requests default to enabled and other providers hide the setting", () => {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	try {
		for (const value of [undefined, null, true, false]) {
			window.document.body.innerHTML = render(
				"OpenRouter",
				"google/gemini-embedding-2",
				value,
			);
			const label = [...window.document.querySelectorAll("label")].find(
				(item) => item.textContent === "Request vector dimensions",
			);
			const toggle = window.document.getElementById(
				label?.getAttribute("for") ?? "",
			);
			expect(toggle?.getAttribute("aria-checked")).toBe(
				String(value !== false),
			);
			const help = window.document.getElementById(
				toggle?.getAttribute("aria-describedby") ?? "",
			);
			expect(help?.textContent).toContain("models with a fixed output size");
			expect(help?.textContent).toContain("model's native size");
			expect(window.document.body.textContent).toContain(
				"The returned vector length must match this Bit's configuration.",
			);
			expect(window.document.body.textContent).not.toContain(
				"The server requests the vector length configured for this Bit.",
			);
		}
		for (const provider of [
			"Internal",
			"CloudflareWorkersAI",
			"OpenAI",
			"Cohere",
			"VoyageAI",
			"AzureOpenAI",
			"HuggingfaceEndpoint",
			"OpenAICompatible",
		]) {
			expect(render(provider)).not.toContain("Request vector dimensions");
		}
	} finally {
		window.happyDOM.abort();
	}
});

test("internal gateway prices cannot be edited as if they override the server tariff", () => {
	const markup = renderToStaticMarkup(
		<HostedEmbeddingConfiguration
			parameters={{
				remote: { implementation: "Internal" },
				pricing: { input_micro_usd_per_million_tokens: 10 },
			}}
			onChange={() => {}}
		/>,
	);
	expect(markup).toContain("tariff configured on the server");
	expect(markup).toContain("Remove unused pricing");
	expect(markup).not.toContain("USD per 1M input tokens");
});

test("editing hosted configuration preserves local settings and byte pricing replaces token pricing", async () => {
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
	const container = window.document.createElement("form");
	window.document.body.append(container);
	const root = createRoot(container as unknown as Element);
	let parameters: Record<string, unknown> = {
		provider: { provider_name: "Local" },
		prefix: { query: "query: " },
		remote: {
			implementation: "OpenAICompatible",
			model_id: "model",
			secret_name: "OLD_KEY",
		},
		pricing: { input_micro_usd_per_million_tokens: 250, request_micro_usd: 4 },
	};
	function Harness() {
		const [value, setValue] = useState(parameters);
		return (
			<HostedEmbeddingConfiguration
				parameters={value}
				onChange={(next) => {
					parameters = next;
					setValue(next);
				}}
			/>
		);
	}
	function inputFor(text: string) {
		const label = [...container.querySelectorAll("label")].find((item) =>
			item.textContent?.startsWith(text),
		);
		const input = window.document.getElementById(
			label?.getAttribute("for") ?? "",
		);
		if (!input) throw new Error(`Missing input: ${text}`);
		return input as unknown as HTMLInputElement;
	}
	async function enter(text: string, value: string) {
		const input = inputFor(text);
		await act(async () => input.focus());
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				window.HTMLInputElement.prototype,
				"value",
			)?.set?.call(input, value);
			input.dispatchEvent(
				new window.Event("input", { bubbles: true }) as unknown as Event,
			);
		});
		await act(async () => input.blur());
	}
	try {
		await act(async () => root.render(<Harness />));
		await enter("API key secret name", "NEW_KEY");
		expect(parameters.remote).toEqual({
			implementation: "OpenAICompatible",
			model_id: "model",
			secret_name: "NEW_KEY",
		});
		const unit = inputFor("Input price unit");
		await act(async () => {
			unit.value = "bytes";
			unit.dispatchEvent(
				new window.Event("change", { bubbles: true }) as unknown as Event,
			);
		});
		expect(parameters.pricing).toEqual({
			input_micro_usd_per_million_bytes: "",
			request_micro_usd: 4,
		});
		await enter("USD per 1M input bytes", "0.000249");
		await enter("USD per 1M input bytes", "");
		expect(inputFor("Input price unit").value).toBe("bytes");
		await enter("USD per 1M input bytes", "0.000249");
		await enter("Maximum input bytes per batch", "4096");
		expect(parameters.pricing).toEqual({
			input_micro_usd_per_million_bytes: 249,
			request_micro_usd: 4,
			max_input_bytes: 4096,
		});
		await act(async () =>
			(
				container.querySelector(
					'[role="switch"]',
				) as unknown as HTMLButtonElement
			)?.click(),
		);
		expect(parameters.remote).toBeNull();
		expect(parameters.pricing).toBeUndefined();
		expect(parameters.provider).toEqual({ provider_name: "Local" });
		expect(parameters.prefix).toEqual({ query: "query: " });
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

test("toggling OpenRouter dimensions preserves the remote configuration and Bit settings", async () => {
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
	const container = window.document.createElement("form");
	window.document.body.append(container);
	const root = createRoot(container as unknown as Element);
	const initial = {
		provider: { provider_name: "Local", model_id: "local-model" },
		prefix: { query: "query: ", paragraph: "passage: " },
		vector_length: 1024,
		input_length: 512,
		pooling: "Mean",
		languages: ["en"],
		remote: {
			implementation: "OpenRouter",
			model_id: "provider/model",
			secret_name: "CUSTOM_OPENROUTER_KEY",
		},
		pricing: { input_micro_usd_per_million_tokens: 250, request_micro_usd: 4 },
	};
	let parameters: Record<string, unknown> = initial;
	function Harness() {
		const [value, setValue] = useState(parameters);
		return (
			<HostedEmbeddingConfiguration
				parameters={value}
				onChange={(next) => {
					parameters = next;
					setValue(next);
				}}
			/>
		);
	}
	try {
		await act(async () => root.render(<Harness />));
		const label = [...container.querySelectorAll("label")].find(
			(item) => item.textContent === "Request vector dimensions",
		);
		const toggle = window.document.getElementById(
			label?.getAttribute("for") ?? "",
		);
		if (!(toggle instanceof window.HTMLElement))
			throw new Error("Missing vector dimensions switch");
		expect(toggle.getAttribute("aria-checked")).toBe("true");
		for (const enabled of [false, true]) {
			await act(async () => toggle.click());
			expect(toggle.getAttribute("aria-checked")).toBe(String(enabled));
			expect(parameters).toEqual({
				...initial,
				remote: { ...initial.remote, supports_dimensions: enabled },
			});
		}
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
