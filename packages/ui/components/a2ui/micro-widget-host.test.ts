/**
 * Tests for the pure host-side flw/1 bridge logic: envelope filtering, props
 * diffing, URL building (desktop/web), rate limiting, height clamping, values
 * key construction, and query correlation.
 */
import { describe, expect, test } from "bun:test";
import {
	MICRO_WIDGET_THEME_TOKENS,
	TokenBucket,
	acceptHostEnvelope,
	buildDesktopMicroWidgetSrc,
	buildWebMicroWidgetPath,
	clampWidgetHeight,
	collectMicroWidgetValueKeys,
	createQueryCorrelator,
	diffMicroWidgetProps,
	generateNonce,
	microWidgetHasInstance,
	microWidgetQuery,
	microWidgetValuesKey,
	readThemeTokens,
	registerMicroWidgetBridge,
	shouldUseHttpSchemeBridge,
} from "./micro-widget-host";

const NONCE = "abc123";
const INSTANCE = "inst-1";

function envelope(overrides: Record<string, unknown> = {}) {
	return {
		protocol: "flw/1",
		nonce: NONCE,
		instanceId: INSTANCE,
		type: "event",
		payload: { name: "pointSelected", payload: { x: 1 } },
		...overrides,
	};
}

describe("generateNonce", () => {
	test("produces 32 hex chars, unique per call", () => {
		const a = generateNonce();
		const b = generateNonce();
		expect(a).toMatch(/^[0-9a-f]{32}$/);
		expect(a).not.toBe(b);
	});
});

describe("acceptHostEnvelope", () => {
	test("accepts a well-formed envelope with matching nonce and instance", () => {
		expect(acceptHostEnvelope(envelope(), INSTANCE, NONCE)).not.toBeNull();
	});

	test("drops non-envelope data", () => {
		expect(acceptHostEnvelope(null, INSTANCE, NONCE)).toBeNull();
		expect(acceptHostEnvelope("hi", INSTANCE, NONCE)).toBeNull();
		expect(
			acceptHostEnvelope({ protocol: "flw/1" }, INSTANCE, NONCE),
		).toBeNull();
		expect(
			acceptHostEnvelope(envelope({ protocol: "flw/2" }), INSTANCE, NONCE),
		).toBeNull();
		expect(
			acceptHostEnvelope(envelope({ type: "not-a-type" }), INSTANCE, NONCE),
		).toBeNull();
	});

	test("drops nonce mismatches", () => {
		expect(
			acceptHostEnvelope(envelope({ nonce: "wrong" }), INSTANCE, NONCE),
		).toBeNull();
		expect(
			acceptHostEnvelope(envelope({ nonce: "" }), INSTANCE, NONCE),
		).toBeNull();
	});

	test("drops instance mismatches", () => {
		expect(
			acceptHostEnvelope(envelope({ instanceId: "other" }), INSTANCE, NONCE),
		).toBeNull();
	});

	test("hello may carry an empty nonce (pre-handshake), but not a wrong one", () => {
		expect(
			acceptHostEnvelope(
				envelope({ type: "hello", nonce: "", payload: {} }),
				INSTANCE,
				NONCE,
			),
		).not.toBeNull();
		expect(
			acceptHostEnvelope(
				envelope({ type: "hello", nonce: NONCE, payload: {} }),
				INSTANCE,
				NONCE,
			),
		).not.toBeNull();
		expect(
			acceptHostEnvelope(
				envelope({ type: "hello", nonce: "wrong", payload: {} }),
				INSTANCE,
				NONCE,
			),
		).toBeNull();
	});
});

describe("diffMicroWidgetProps", () => {
	test("returns null when nothing changed (deep-equal via JSON)", () => {
		const prev = { title: "Sales", rows: [{ x: "a", y: 1 }] };
		const next = { title: "Sales", rows: [{ x: "a", y: 1 }] };
		expect(diffMicroWidgetProps(prev, next)).toBeNull();
	});

	test("returns only changed and added keys", () => {
		const prev = { title: "Sales", limit: 50 };
		const next = { title: "Q3 Sales", limit: 50, variant: "line" };
		expect(diffMicroWidgetProps(prev, next)).toEqual({
			title: "Q3 Sales",
			variant: "line",
		});
	});

	test("detects nested changes even with fresh object identity", () => {
		const prev = { rows: [{ x: "a", y: 1 }] };
		const next = { rows: [{ x: "a", y: 2 }] };
		expect(diffMicroWidgetProps(prev, next)).toEqual({
			rows: [{ x: "a", y: 2 }],
		});
	});

	test("removed keys are patched to undefined", () => {
		expect(diffMicroWidgetProps({ gone: 1, kept: 2 }, { kept: 2 })).toEqual({
			gone: undefined,
		});
	});
});

describe("URL building", () => {
	test("desktop custom-protocol form", () => {
		expect(
			buildDesktopMicroWidgetSrc({
				packageId: "com.example.sales",
				bundleHash: "deadbeef",
				widgetId: "sales-chart",
				useHttpBridge: false,
			}),
		).toBe(
			"flow-widget://localhost/com.example.sales/deadbeef/widgets/sales-chart/index.html",
		);
	});

	test("desktop http bridge form (Windows WebView2 / Android)", () => {
		expect(
			buildDesktopMicroWidgetSrc({
				packageId: "com.example.sales",
				bundleHash: "deadbeef",
				widgetId: "sales-chart",
				useHttpBridge: true,
			}),
		).toBe(
			"http://flow-widget.localhost/com.example.sales/deadbeef/widgets/sales-chart/index.html",
		);
	});

	test("segments are URI-encoded but slashes between segments survive", () => {
		expect(
			buildDesktopMicroWidgetSrc({
				packageId: "a b",
				bundleHash: "h#1",
				widgetId: "w/1",
				useHttpBridge: false,
			}),
		).toBe("flow-widget://localhost/a%20b/h%231/widgets/w%2F1/index.html");
	});

	test("web registry path", () => {
		expect(
			buildWebMicroWidgetPath("com.example.sales", "1.2.0", "sales-chart"),
		).toBe(
			"registry/package/com.example.sales/widget-asset/1.2.0/widgets/sales-chart/index.html",
		);
	});

	test("http bridge platform detection", () => {
		expect(
			shouldUseHttpSchemeBridge(
				"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
			),
		).toBeTrue();
		expect(
			shouldUseHttpSchemeBridge("Mozilla/5.0 (Linux; Android 14; Pixel 8)"),
		).toBeTrue();
		expect(
			shouldUseHttpSchemeBridge(
				"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
			),
		).toBeFalse();
		expect(
			shouldUseHttpSchemeBridge("Mozilla/5.0 (X11; Linux x86_64)"),
		).toBeFalse();
	});
});

describe("values key construction", () => {
	test("microWidgetValuesKey", () => {
		expect(microWidgetValuesKey("inst-1")).toBe("inst-1/values");
	});

	test("collectMicroWidgetValueKeys picks only micro widget instances", () => {
		const keys = collectMicroWidgetValueKeys({
			a: { component: { type: "microWidgetInstance", instanceId: "inst-a" } },
			b: { component: { type: "widgetInstance", instanceId: "inst-b" } },
			c: { component: { type: "text" } },
			d: { component: { type: "microWidgetInstance", instanceId: "" } },
		});
		expect(keys).toEqual(new Set(["inst-a/values"]));
	});

	test("collectMicroWidgetValueKeys tolerates undefined input", () => {
		expect(collectMicroWidgetValueKeys(undefined)).toEqual(new Set());
	});
});

describe("TokenBucket", () => {
	test("allows a burst up to capacity, then throttles", () => {
		const bucket = new TokenBucket(3, 3);
		const now = 1_000_000;
		expect(bucket.tryTake(now)).toBeTrue();
		expect(bucket.tryTake(now)).toBeTrue();
		expect(bucket.tryTake(now)).toBeTrue();
		expect(bucket.tryTake(now)).toBeFalse();
	});

	test("refills over time at the configured rate", () => {
		const bucket = new TokenBucket(2, 2);
		const now = 1_000_000;
		expect(bucket.tryTake(now)).toBeTrue();
		expect(bucket.tryTake(now)).toBeTrue();
		expect(bucket.tryTake(now)).toBeFalse();
		// 500ms at 2/s refills one token.
		expect(bucket.tryTake(now + 500)).toBeTrue();
		expect(bucket.tryTake(now + 500)).toBeFalse();
	});

	test("never exceeds capacity after a long idle period", () => {
		const bucket = new TokenBucket(2, 2);
		const now = 1_000_000;
		bucket.tryTake(now);
		expect(bucket.tryTake(now + 60_000)).toBeTrue();
		expect(bucket.tryTake(now + 60_000)).toBeTrue();
		expect(bucket.tryTake(now + 60_000)).toBeFalse();
	});
});

describe("clampWidgetHeight", () => {
	test("clamps to maxHeight when resizing beyond it", () => {
		expect(clampWidgetHeight(900, { maxHeight: 600 })).toBe(600);
	});

	test("passes through sane heights (ceiled)", () => {
		expect(clampWidgetHeight(240.4, { maxHeight: 600 })).toBe(241);
		expect(clampWidgetHeight(240, undefined)).toBe(240);
	});

	test("falls back to the default height for garbage input", () => {
		expect(clampWidgetHeight(Number.NaN, { defaultHeight: 320 })).toBe(320);
		expect(clampWidgetHeight(-5, { defaultHeight: 100 })).toBe(100);
	});
});

describe("readThemeTokens", () => {
	test("reads whitelisted tokens and skips empty values", () => {
		const source: Record<string, string> = {
			"--background": " oklch(1 0 0) ",
			"--primary": "red",
		};
		const tokens = readThemeTokens((name) => source[name] ?? "");
		expect(tokens).toEqual({
			"--background": "oklch(1 0 0)",
			"--primary": "red",
		});
	});

	test("whitelist covers the SDK token set", () => {
		expect(MICRO_WIDGET_THEME_TOKENS).toContain("--background");
		expect(MICRO_WIDGET_THEME_TOKENS).toContain("--radius");
		expect(MICRO_WIDGET_THEME_TOKENS).toContain("--font-sans");
	});
});

describe("query correlation", () => {
	test("resolves a query by queryId", async () => {
		const posted: { queryId: string; name: string; args: unknown }[] = [];
		const correlator = createQueryCorrelator((payload) => posted.push(payload));

		const pending = correlator.request("getSelection", { limit: 5 });
		expect(posted).toHaveLength(1);
		expect(posted[0].name).toBe("getSelection");

		correlator.handleResult({
			queryId: posted[0].queryId,
			ok: true,
			value: { rows: [] },
		});
		await expect(pending).resolves.toEqual({ rows: [] });
		correlator.dispose();
	});

	test("rejects on widget-reported errors", async () => {
		const posted: { queryId: string }[] = [];
		const correlator = createQueryCorrelator((payload) => posted.push(payload));
		const pending = correlator.request("getValue", undefined);
		correlator.handleResult({
			queryId: posted[0].queryId,
			ok: false,
			error: "boom",
		});
		await expect(pending).rejects.toThrow("boom");
		correlator.dispose();
	});

	test("rejects after the timeout", async () => {
		const correlator = createQueryCorrelator(() => {});
		await expect(correlator.request("getValue", undefined, 10)).rejects.toThrow(
			/timed out/,
		);
		correlator.dispose();
	});

	test("ignores unknown queryIds", () => {
		const correlator = createQueryCorrelator(() => {});
		expect(() =>
			correlator.handleResult({ queryId: "nope", ok: true, value: 1 }),
		).not.toThrow();
		correlator.dispose();
	});

	test("dispose rejects everything in flight", async () => {
		const correlator = createQueryCorrelator(() => {});
		const pending = correlator.request("getValue", undefined, 10_000);
		correlator.dispose();
		await expect(pending).rejects.toThrow(/disposed/);
	});
});

describe("live bridge registry", () => {
	test("register / query / unregister round-trip", async () => {
		const unregister = registerMicroWidgetBridge("reg-1", {
			query: async (name, args) => ({ name, args }),
		});
		expect(microWidgetHasInstance("reg-1")).toBeTrue();
		await expect(microWidgetQuery("reg-1", "getValue", 7)).resolves.toEqual({
			name: "getValue",
			args: 7,
		});
		unregister();
		expect(microWidgetHasInstance("reg-1")).toBeFalse();
		await expect(microWidgetQuery("reg-1", "getValue", 7)).rejects.toThrow(
			/No live micro widget instance/,
		);
	});

	test("a stale unregister does not remove a newer registration", () => {
		const first = registerMicroWidgetBridge("reg-2", {
			query: async () => 1,
		});
		registerMicroWidgetBridge("reg-2", { query: async () => 2 });
		first();
		expect(microWidgetHasInstance("reg-2")).toBeTrue();
	});
});
