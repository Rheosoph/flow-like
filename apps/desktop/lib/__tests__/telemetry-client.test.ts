import type { ITelemetryCapturedEvent } from "@flow-like/flow-like-ui/lib/telemetry/capture";
import {
	type ITelemetryClient,
	createTelemetryClient,
} from "@flow-like/flow-like-ui/lib/telemetry/client";
import type { IApiState } from "@flow-like/flow-like-ui/state/backend-state/api-state";
import type { IProfile } from "@flow-like/flow-like-ui/types";
import { afterEach, describe, expect, test, vi } from "vitest";

const profile = {
	bits: [],
	created: "",
	updated: "",
	name: "test",
} as IProfile;

function capturedEvent(name: string): ITelemetryCapturedEvent {
	return { name, client_ts: "2026-07-26T00:00:00.000Z" };
}

function capturedEvents(count: number) {
	return Array.from({ length: count }, (_, index) =>
		capturedEvent(`event_${index}`),
	);
}

interface ClientHarness {
	client: ITelemetryClient;
	post: ReturnType<typeof vi.fn>;
	enabled: { value: boolean };
	profileRef: { value: IProfile | undefined };
}

let harnesses: ClientHarness[] = [];

function createHarness(
	overrides: Partial<Parameters<typeof createTelemetryClient>[0]> = {},
): ClientHarness {
	const post = vi.fn().mockResolvedValue({ accepted: 0 });
	const enabled = { value: true };
	const profileRef = { value: profile as IProfile | undefined };
	const client = createTelemetryClient({
		apiState: { post } as unknown as IApiState,
		getProfile: () => profileRef.value,
		isEnabled: () => enabled.value,
		getAnonId: () => "anon-1234",
		source: "desktop",
		appVersion: "1.2.3",
		platform: "linux",
		...overrides,
	});
	const harness = { client, post, enabled, profileRef };
	harnesses.push(harness);
	return harness;
}

afterEach(() => {
	for (const harness of harnesses) harness.client.dispose();
	harnesses = [];
});

describe("telemetry client", () => {
	test("flushes queued events with the snake_case ingest body", async () => {
		const { client, post } = createHarness();
		client.sink(capturedEvent("page_view"));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(1);
		expect(post).toHaveBeenCalledWith(profile, "telemetry/events", {
			anon_id: "anon-1234",
			source: "desktop",
			app_version: "1.2.3",
			platform: "linux",
			events: [capturedEvent("page_view")],
		});
	});

	test("defaults app_version and platform to null", async () => {
		const { client, post } = createHarness({
			appVersion: undefined,
			platform: undefined,
		});
		client.enqueue([capturedEvent("app_started")]);
		await client.flush();

		expect(post.mock.calls[0]?.[2]).toMatchObject({
			app_version: null,
			platform: null,
		});
	});

	test("splits large queues into batches of at most 50 per request", async () => {
		const { client, post } = createHarness();
		client.enqueue(capturedEvents(120));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(3);
		const sizes = post.mock.calls.map((call) => call[2].events.length);
		expect(sizes).toEqual([50, 50, 20]);
	});

	test("sends at most 5 requests per flush and keeps the rest queued", async () => {
		const { client, post } = createHarness({ maxQueueSize: 300 });
		client.enqueue(capturedEvents(260));
		await client.flush();
		expect(post).toHaveBeenCalledTimes(5);

		await client.flush();
		expect(post).toHaveBeenCalledTimes(6);
		expect(post.mock.calls[5]?.[2].events).toHaveLength(10);
	});

	test("caps the queue by dropping the oldest events", async () => {
		const { client, post } = createHarness({ maxQueueSize: 4 });
		client.enqueue(capturedEvents(6));
		await client.flush();

		expect(
			post.mock.calls[0]?.[2].events.map(
				(event: ITelemetryCapturedEvent) => event.name,
			),
		).toEqual(["event_2", "event_3", "event_4", "event_5"]);
	});

	test("drops events while disabled", async () => {
		const { client, post, enabled } = createHarness();
		enabled.value = false;
		client.sink(capturedEvent("dropped"));
		client.enqueue([capturedEvent("also_dropped")]);

		enabled.value = true;
		await client.flush();
		expect(post).not.toHaveBeenCalled();
	});

	test("does not flush without a profile or anon id", async () => {
		const { client, post, profileRef } = createHarness();
		client.enqueue([capturedEvent("waiting")]);
		profileRef.value = undefined;
		await client.flush();
		expect(post).not.toHaveBeenCalled();

		profileRef.value = profile;
		await client.flush();
		expect(post).toHaveBeenCalledTimes(1);
	});

	test("re-queues a failed batch once and never throws", async () => {
		const { client, post } = createHarness();
		post.mockRejectedValueOnce(new Error("offline"));
		client.enqueue([capturedEvent("retry_me")]);

		await expect(client.flush()).resolves.toBeUndefined();
		expect(post).toHaveBeenCalledTimes(1);

		await client.flush();
		expect(post).toHaveBeenCalledTimes(2);
		expect(post.mock.calls[1]?.[2].events).toEqual([capturedEvent("retry_me")]);
	});

	test("drops a batch after its single retry also fails", async () => {
		const { client, post } = createHarness();
		post.mockRejectedValue(new Error("offline"));
		client.enqueue([capturedEvent("doomed")]);

		await client.flush();
		await client.flush();
		expect(post).toHaveBeenCalledTimes(2);

		post.mockResolvedValue({ accepted: 0 });
		await client.flush();
		expect(post).toHaveBeenCalledTimes(2);
	});

	test("dispose drops the queue and stops flushing", async () => {
		const { client, post } = createHarness();
		client.enqueue([capturedEvent("gone")]);
		client.dispose();
		await client.flush();
		expect(post).not.toHaveBeenCalled();
	});

	test("clear empties the queue so the next flush sends nothing", async () => {
		const { client, post } = createHarness();
		post.mockRejectedValueOnce(new Error("offline"));
		client.enqueue([capturedEvent("stale")]);
		await client.flush();
		expect(post).toHaveBeenCalledTimes(1);

		client.clear();
		await client.flush();
		expect(post).toHaveBeenCalledTimes(1);

		client.enqueue([capturedEvent("fresh")]);
		await client.flush();
		expect(post).toHaveBeenCalledTimes(2);
		expect(post.mock.calls[1]?.[2].events).toEqual([capturedEvent("fresh")]);
	});

	test("clear resets the single-retry flag", async () => {
		const { client, post } = createHarness();
		post.mockRejectedValueOnce(new Error("offline"));
		client.enqueue([capturedEvent("first")]);
		await client.flush();

		client.clear();
		post.mockRejectedValueOnce(new Error("offline"));
		client.enqueue([capturedEvent("second")]);
		await client.flush();

		await client.flush();
		expect(post).toHaveBeenCalledTimes(3);
		expect(post.mock.calls[2]?.[2].events).toEqual([capturedEvent("second")]);
	});
});

describe("telemetry client unload beacon", () => {
	function stubWindow() {
		const listeners = new Map<string, () => void>();
		vi.stubGlobal("window", {
			addEventListener: (name: string, handler: () => void) => {
				listeners.set(name, handler);
			},
			removeEventListener: (name: string) => {
				listeners.delete(name);
			},
		});
		return listeners;
	}

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	test("hands queued events to the beacon on pagehide and drops them", async () => {
		const listeners = stubWindow();
		const beacon = vi.fn().mockReturnValue(true);
		const { client, post } = createHarness({ beacon });
		client.enqueue([capturedEvent("page_view")]);

		listeners.get("pagehide")?.();

		expect(beacon).toHaveBeenCalledTimes(1);
		expect(beacon).toHaveBeenCalledWith({
			anon_id: "anon-1234",
			source: "desktop",
			app_version: "1.2.3",
			platform: "linux",
			events: [capturedEvent("page_view")],
		});
		await client.flush();
		expect(post).not.toHaveBeenCalled();
	});

	test("falls back to flush when the beacon rejects the body", async () => {
		const listeners = stubWindow();
		const beacon = vi.fn().mockReturnValue(false);
		const { post, client } = createHarness({ beacon });
		client.enqueue([capturedEvent("page_view")]);

		listeners.get("pagehide")?.();

		await vi.waitFor(() => expect(post).toHaveBeenCalledTimes(1));
		expect(beacon).toHaveBeenCalledTimes(1);
		expect(post.mock.calls[0]?.[2].events).toEqual([
			capturedEvent("page_view"),
		]);
	});

	test("falls back to flush when no beacon is configured", async () => {
		const listeners = stubWindow();
		const { post, client } = createHarness();
		client.enqueue([capturedEvent("page_view")]);

		listeners.get("pagehide")?.();

		await vi.waitFor(() => expect(post).toHaveBeenCalledTimes(1));
	});
});
