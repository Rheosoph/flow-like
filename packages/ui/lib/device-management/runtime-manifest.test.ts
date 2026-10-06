import { describe, expect, test } from "bun:test";
import { emptyModels, overviewOf } from "./model/__fixtures__/sample-models";
import { MODEL_RUNTIME_MANIFEST_MAX_BYTES } from "./models";
import { prepareRuntimeInstall } from "./runtime-manifest";
import type { ManagementCall } from "./telemetry";

const request = {
	kind: "install_runtime",
	runtime: "llamacpp",
	backend: "cpu",
} as const;
const features = { model_host: 1, model_runtime_manifest: 1 } as const;
const url = "https://releases.example.com/runtimes/aarch64-apple-darwin.jws";
const compact = "header.payload.signature";

function device(manifestUrl: string | undefined = url) {
	const sent: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command) => {
		sent.push(command);
		return {
			operation_id: "read",
			state: "completed",
			result: {
				...overviewOf(emptyModels()),
				runtime_manifest_url: manifestUrl,
			},
		};
	};
	return { call, sent };
}

function fetcher(response: () => Response | Promise<Response>) {
	const calls: { input: RequestInfo | URL; options?: RequestInit }[] = [];
	const fetch = (async (input, options) => {
		calls.push({ input, options });
		return response();
	}) as typeof globalThis.fetch;
	return { fetch, calls };
}

describe("runtime manifest forwarding", () => {
	test("carries the enrolled manifest without credentials and leaves verification to the device", async () => {
		const agent = device();
		const download = fetcher(() => new Response(`${compact}\n`));
		expect(
			await prepareRuntimeInstall(
				agent.call,
				features,
				request,
				download.fetch,
			),
		).toEqual({
			...request,
			manifest_jws: compact,
		});
		expect(agent.sent).toEqual([
			{ type: "models", request: { kind: "overview" } },
		]);
		expect(download.calls[0]).toMatchObject({
			input: url,
			options: { credentials: "omit", redirect: "error", cache: "no-store" },
		});
	});

	test("older agents receive no extra read or unknown request field", async () => {
		const agent = device();
		const download = fetcher(() => new Response(compact));
		expect(
			await prepareRuntimeInstall(
				agent.call,
				{ model_host: 1 },
				request,
				download.fetch,
			),
		).toEqual(request);
		expect(agent.sent).toEqual([]);
		expect(download.calls).toEqual([]);
	});

	test.each([
		undefined,
		"http://releases.example.com/runtime.jws",
		"https://user:secret@releases.example.com/runtime.jws",
		"https://releases.example.com/runtime.jws?token=secret",
	])(
		"a missing or non-public manifest source is not fetched: %s",
		async (source) => {
			const agent = device();
			const call: ManagementCall = async (command, operation) => ({
				...(await agent.call(command, operation)),
				result: { ...overviewOf(emptyModels()), runtime_manifest_url: source },
			});
			const download = fetcher(() => new Response(compact));
			expect(
				await prepareRuntimeInstall(call, features, request, download.fetch),
			).toEqual(request);
			expect(download.calls).toEqual([]);
		},
	);

	test.each([
		() => new Response("missing", { status: 404 }),
		() => new Response("<html>Unavailable</html>"),
		() => new Response("header.unicodé.signature"),
		() =>
			new Response(compact, {
				headers: {
					"content-length": String(MODEL_RUNTIME_MANIFEST_MAX_BYTES + 1),
				},
			}),
		() => Promise.reject(new TypeError("Browser cannot reach the release CDN")),
	])(
		"a browser download failure keeps the agent's own download available",
		async (response) => {
			expect(
				await prepareRuntimeInstall(
					device().call,
					features,
					request,
					fetcher(response).fetch,
				),
			).toEqual(request);
		},
	);

	test("bounds chunked downloads before forwarding and cancels the response", async () => {
		let cancelled = false;
		const body = new ReadableStream<Uint8Array>({
			pull(controller) {
				controller.enqueue(new Uint8Array(1024).fill(65));
			},
			cancel() {
				cancelled = true;
			},
		});
		expect(
			await prepareRuntimeInstall(
				device().call,
				features,
				request,
				fetcher(() => new Response(body)).fetch,
			),
		).toEqual(request);
		expect(cancelled).toBe(true);
	});

	test("accepts the exact ASCII envelope allowance", async () => {
		const largest = `h.${"p".repeat(MODEL_RUNTIME_MANIFEST_MAX_BYTES - 4)}.s`;
		expect(largest.length).toBe(MODEL_RUNTIME_MANIFEST_MAX_BYTES);
		expect(
			(
				await prepareRuntimeInstall(
					device().call,
					features,
					request,
					fetcher(() => new Response(largest)).fetch,
				)
			).manifest_jws,
		).toBe(largest);
	});
});
