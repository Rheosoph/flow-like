import {
	afterAll,
	beforeEach,
	describe,
	expect,
	it,
	mock,
	spyOn,
} from "bun:test";
import type { IChannelHandle } from "../../lib/schema/channel";
import type { ElementSource } from "./element-materializer";
import {
	handleElementsRequestMessage,
	parseElementsRequestMessage,
} from "./elements-request-handler";

const channel: IChannelHandle = {
	channel_id: "run-1",
	request_id: "req-1",
	expires_at: 4_102_444_800,
	transport: { type: "in_process" },
};

const source: ElementSource = {
	surfaceId: "page-1",
	components: {},
	storedValues: {},
};

const materialized = { "page-1/title": { id: "title" } };

function request(overrides: Record<string, unknown> = {}) {
	return {
		type: "requestElements",
		request_id: "req-1",
		selectors: ["title"],
		timeout_ms: 5000,
		channel,
		...overrides,
	};
}

const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("parseElementsRequestMessage", () => {
	it("parses the snake_case wire form", () => {
		expect(parseElementsRequestMessage(request())).toEqual({
			requestId: "req-1",
			selectors: ["title"],
			timeoutMs: 5000,
			channel,
		});
	});

	it("parses the normalized camelCase form", () => {
		const parsed = parseElementsRequestMessage({
			type: "requestElements",
			requestId: "req-2",
			selectors: ["type:Button"],
			timeoutMs: 250,
			channel,
		});
		expect(parsed).toEqual({
			requestId: "req-2",
			selectors: ["type:Button"],
			timeoutMs: 250,
			channel,
		});
	});

	it("defaults the timeout when missing or invalid", () => {
		expect(
			parseElementsRequestMessage(request({ timeout_ms: -1 }))?.timeoutMs,
		).toBe(10_000);
		expect(
			parseElementsRequestMessage(request({ timeout_ms: undefined }))
				?.timeoutMs,
		).toBe(10_000);
	});

	it("keeps only string selectors and tolerates a missing list", () => {
		expect(
			parseElementsRequestMessage(request({ selectors: ["a", 1, null, "b"] }))
				?.selectors,
		).toEqual(["a", "b"]);
		expect(
			parseElementsRequestMessage(request({ selectors: undefined }))?.selectors,
		).toEqual([]);
	});

	it("drops a missing or malformed channel instead of the whole request", () => {
		const missing = parseElementsRequestMessage(request({ channel: null }));
		expect(missing?.requestId).toBe("req-1");
		expect(missing?.channel).toBeNull();

		const malformed = parseElementsRequestMessage(
			request({ channel: { channel_id: "run-1" } }),
		);
		expect(malformed?.requestId).toBe("req-1");
		expect(malformed?.channel).toBeNull();
	});

	it("returns null for the legacy element_ids shape", () => {
		expect(
			parseElementsRequestMessage({
				type: "requestElements",
				element_ids: ["title"],
			}),
		).toBeNull();
	});

	it("rejects other message types and malformed requests", () => {
		expect(parseElementsRequestMessage(null)).toBeNull();
		expect(parseElementsRequestMessage("requestElements")).toBeNull();
		expect(parseElementsRequestMessage({ type: "widgetQuery" })).toBeNull();
		expect(parseElementsRequestMessage(request({ request_id: 42 }))).toBeNull();
		expect(parseElementsRequestMessage(request({ request_id: "" }))).toBeNull();
	});
});

describe("handleElementsRequestMessage", () => {
	const reply = mock(async (_channel: IChannelHandle, _value: unknown) => {});
	const materialize = mock(
		(_source: ElementSource, _selectors: readonly string[]) => materialized,
	);
	const warn = spyOn(console, "warn");
	const options = { materialize, reply };
	let sequence = 0;
	const uniqueRequest = (overrides: Record<string, unknown> = {}) =>
		request({ request_id: `req-${++sequence}`, ...overrides });

	beforeEach(() => {
		reply.mockClear();
		reply.mockImplementation(async () => {});
		materialize.mockClear();
		materialize.mockImplementation(() => materialized);
		warn.mockClear();
		warn.mockImplementation(() => {});
	});

	afterAll(() => warn.mockRestore());

	it("returns false for other message types", () => {
		expect(
			handleElementsRequestMessage(
				{ type: "widgetQuery" },
				() => source,
				options,
			),
		).toBe(false);
		expect(handleElementsRequestMessage(null, () => source, options)).toBe(
			false,
		);
		expect(reply).not.toHaveBeenCalled();
	});

	it("swallows the legacy shape without answering", async () => {
		expect(
			handleElementsRequestMessage(
				{ type: "requestElements", element_ids: ["title"] },
				() => source,
				options,
			),
		).toBe(true);
		await flush();
		expect(reply).not.toHaveBeenCalled();
		expect(warn).not.toHaveBeenCalled();
	});

	it("warns and stays silent when the message carries no channel", async () => {
		expect(
			handleElementsRequestMessage(
				uniqueRequest({ channel: null }),
				() => source,
				options,
			),
		).toBe(true);
		await flush();
		expect(reply).not.toHaveBeenCalled();
		expect(warn).toHaveBeenCalledTimes(1);
	});

	it("replies ok with the materialized map", async () => {
		const message = uniqueRequest({ selectors: ["title", "type:Button"] });
		expect(handleElementsRequestMessage(message, () => source, options)).toBe(
			true,
		);
		await flush();
		expect(materialize).toHaveBeenCalledTimes(1);
		expect(materialize.mock.calls[0]).toEqual([
			source,
			["title", "type:Button"],
		]);
		expect(reply).toHaveBeenCalledTimes(1);
		expect(reply.mock.calls[0]).toEqual([
			channel,
			{ ok: true, elements: materialized },
		]);
	});

	it("replies an error when there is no live surface", async () => {
		handleElementsRequestMessage(uniqueRequest(), () => null, options);
		await flush();
		expect(materialize).not.toHaveBeenCalled();
		expect(reply.mock.calls[0]).toEqual([
			channel,
			{ ok: false, error: "no live surface for this run" },
		]);
	});

	it("replies an error when materializing throws", async () => {
		materialize.mockImplementation(() => {
			throw new Error("boom");
		});
		handleElementsRequestMessage(uniqueRequest(), () => source, options);
		await flush();
		expect(reply.mock.calls[0]).toEqual([
			channel,
			{ ok: false, error: "boom" },
		]);
	});

	it("dedupes concurrent duplicates of one request id", async () => {
		let release: () => void = () => {};
		reply.mockImplementation(
			() =>
				new Promise<void>((resolve) => {
					release = resolve;
				}),
		);
		const message = uniqueRequest();
		expect(handleElementsRequestMessage(message, () => source, options)).toBe(
			true,
		);
		expect(handleElementsRequestMessage(message, () => source, options)).toBe(
			true,
		);
		expect(reply).toHaveBeenCalledTimes(1);

		release();
		await flush();
		handleElementsRequestMessage(message, () => source, options);
		expect(reply).toHaveBeenCalledTimes(2);
	});

	it("clears the in-flight id after delivery", async () => {
		const message = uniqueRequest();
		handleElementsRequestMessage(message, () => source, options);
		await flush();
		handleElementsRequestMessage(message, () => source, options);
		await flush();
		expect(reply).toHaveBeenCalledTimes(2);
	});

	it("clears the in-flight id and warns when delivery fails", async () => {
		reply.mockImplementation(async () => {
			throw new Error("offline");
		});
		const message = uniqueRequest();
		handleElementsRequestMessage(message, () => source, options);
		await flush();
		expect(warn).toHaveBeenCalledTimes(1);
		handleElementsRequestMessage(message, () => source, options);
		await flush();
		expect(reply).toHaveBeenCalledTimes(2);
	});
});
