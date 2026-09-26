/**
 * Tests for the pure host-side flw/1 bridge logic: envelope filtering, props
 * diffing, URL building (desktop/web), rate limiting, height clamping, values
 * key construction, and query correlation.
 */
import { describe, expect, test } from "bun:test";
import {
	MICRO_WIDGET_EVENT_BURST,
	MICRO_WIDGET_EVENT_RATE_PER_SECOND,
	MICRO_WIDGET_THEME_TOKENS,
	TokenBucket,
	acceptHostEnvelope,
	buildDesktopMicroWidgetFrameSrc,
	buildDesktopMicroWidgetSrc,
	buildWebMicroWidgetFramePath,
	buildWebMicroWidgetPath,
	clampWidgetHeight,
	collectMicroWidgetValueKeys,
	createQueryCorrelator,
	diffMicroWidgetProps,
	generateNonce,
	isMicroWidgetServingUrl,
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

const HASH = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const DESKTOP_GRANT = "0f".repeat(32);
const WEB_GRANT = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";

describe("grant-aware frame URLs", () => {
	test("desktop custom-protocol form carries the grant or the baseline segment", () => {
		expect(
			buildDesktopMicroWidgetFrameSrc({
				packageId: "com.example.maps",
				bundleHash: HASH,
				widgetId: "live-map",
				grant: DESKTOP_GRANT,
				useHttpBridge: false,
			}),
		).toBe(
			`flow-widget://localhost/com.example.maps/${HASH}/frame/live-map/${DESKTOP_GRANT}`,
		);
		expect(
			buildDesktopMicroWidgetFrameSrc({
				packageId: "com.example.maps",
				bundleHash: HASH,
				widgetId: "live-map",
				grant: null,
				useHttpBridge: false,
			}),
		).toBe(`flow-widget://localhost/com.example.maps/${HASH}/frame/live-map/0`);
	});

	test("desktop http bridge form (Windows WebView2 / Android)", () => {
		expect(
			buildDesktopMicroWidgetFrameSrc({
				packageId: "com.example.maps",
				bundleHash: HASH,
				widgetId: "live-map",
				grant: null,
				useHttpBridge: true,
			}),
		).toBe(
			`http://flow-widget.localhost/com.example.maps/${HASH}/frame/live-map/0`,
		);
	});

	test("web path uses the widget-sandbox route", () => {
		expect(
			buildWebMicroWidgetFramePath({
				packageId: "com.example.maps",
				packageVersion: "1.2.0",
				widgetId: "live-map",
				grant: WEB_GRANT,
			}),
		).toBe(
			`registry/package/com.example.maps/widget-sandbox/1.2.0/frame/live-map/${WEB_GRANT}`,
		);
		expect(
			buildWebMicroWidgetFramePath({
				packageId: "a b",
				packageVersion: "1.0.0+build/1",
				widgetId: "live-map",
				grant: null,
			}),
		).toBe(
			"registry/package/a%20b/widget-sandbox/1.0.0%2Bbuild%2F1/frame/live-map/0",
		);
	});

	test("never emit a query, whatever the grant", () => {
		const urls = [
			buildDesktopMicroWidgetFrameSrc({
				packageId: "com.example.maps",
				bundleHash: HASH,
				widgetId: "live-map",
				grant: DESKTOP_GRANT,
				useHttpBridge: false,
			}),
			buildWebMicroWidgetFramePath({
				packageId: "com.example.maps",
				packageVersion: "1.2.0",
				widgetId: "live-map",
				grant: WEB_GRANT,
			}),
		];
		for (const url of urls) {
			expect(url).not.toContain("?");
			expect(url).not.toContain("downloads");
		}
	});

	test("malformed grants are refused instead of spliced into the path", () => {
		const desktop = (grant: string) => () =>
			buildDesktopMicroWidgetFrameSrc({
				packageId: "com.example.maps",
				bundleHash: HASH,
				widgetId: "live-map",
				grant,
				useHttpBridge: false,
			});
		const web = (grant: string) => () =>
			buildWebMicroWidgetFramePath({
				packageId: "com.example.maps",
				packageVersion: "1.2.0",
				widgetId: "live-map",
				grant,
			});
		for (const grant of [
			"",
			"0",
			"../../widgets/other/index.0.html",
			DESKTOP_GRANT.toUpperCase(),
			WEB_GRANT,
		]) {
			expect(desktop(grant)).toThrow(/malformed grant/);
		}
		for (const grant of [
			"",
			"0",
			DESKTOP_GRANT,
			"a.b",
			"a.b.c?downloads=1",
			"a.b/../c.d",
			`${"a".repeat(2048)}.b.c`,
			`${WEB_GRANT}~eyJ4IjpbXX0`,
		]) {
			expect(web(grant)).toThrow(/malformed grant/);
		}
	});

	test("web grants carry the runtime component of their mint after a tilde", () => {
		const web = (grant: string | null, runtime: string | null | undefined) =>
			buildWebMicroWidgetFramePath({
				packageId: "com.example.maps",
				packageVersion: "1.2.0",
				widgetId: "live-map",
				grant,
				runtime,
			});
		const runtime = "eyJ0aWxlVXJsIjpbImh0dHBzOi8vYS5leGFtcGxlLmNvbSJdfQ";
		expect(web(WEB_GRANT, runtime)).toBe(
			`registry/package/com.example.maps/widget-sandbox/1.2.0/frame/live-map/${WEB_GRANT}~${runtime}`,
		);
		expect(web(WEB_GRANT, null)).toEndWith(`/frame/live-map/${WEB_GRANT}`);
		expect(web(WEB_GRANT, "a".repeat(1366))).toEndWith(
			`${WEB_GRANT}~${"a".repeat(1366)}`,
		);
		for (const malformed of [
			"",
			"a",
			"a".repeat(1367),
			"abcde",
			"eyJ4Ijo=",
			"a/b",
			"a~b",
			"a.b",
			"a%2Fb",
		]) {
			expect(() => web(WEB_GRANT, malformed)).toThrow(
				/malformed runtime component/,
			);
		}
		expect(() => web(null, runtime)).toThrow(/malformed runtime component/);
	});
});

describe("isMicroWidgetServingUrl", () => {
	test("recognizes every widget serving form", () => {
		for (const url of [
			`flow-widget://localhost/com.example.maps/${HASH}/frame/live-map/0`,
			"FLOW-WIDGET://localhost/x",
			" flow-widget://localhost/x",
			`http://flow-widget.localhost/com.example.maps/${HASH}/frame/live-map/0`,
			"https://flow-widget.localhost/x",
			"http://sub.flow-widget.localhost/x",
			"http://FLOW-WIDGET.localhost./x",
			"https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/frame/live-map/0",
			"https://app.flow-like.com/api/v1/registry/package/com.example.maps/widget-asset/1.2.0/widgets/live-map/index.html",
			"https://api.flow-like.com/api/v1/registry//package/x/widget-sandbox/1/frame/w/0",
			"https://api.flow-like.com/api/v1/Registry/Package/x/Widget-Sandbox/1/frame/w/0",
			"https://api.flow-like.com/api/v1/registry/package/x/widget%2Dsandbox/1/frame/w/0",
			"https://api.flow-like.com/api/v1/registry%2Fpackage%2Fx%2Fwidget-asset%2F1/a",
			"https://api.flow-like.com/api/v1/registry/package/x/widget%252Dsandbox/1/a",
			"https://api.flow-like.com/api/v1/registry/package/x/widget-sandbox/%252e%252e/y",
			"https://api.flow-like.com/api/v1/registry/package/x/widget-sandbox",
			"https://api.flow-like.com/api/v1/registry/package/x/%E0%A4%A",
		]) {
			expect({ url, serving: isMicroWidgetServingUrl(url) }).toEqual({
				url,
				serving: true,
			});
		}
	});

	test("resolves relative paths against the page", () => {
		expect(
			isMicroWidgetServingUrl(
				"../api/v1/registry/package/x/widget-sandbox/1/frame/w/0",
				"https://app.flow-like.com/use/page",
			),
		).toBeTrue();
		expect(
			isMicroWidgetServingUrl(
				"apps/x/index.html",
				"https://app.flow-like.com/use",
			),
		).toBeFalse();
	});

	test("ordinary pages are not serving URLs", () => {
		for (const url of [
			"https://www.youtube.com/embed/abc",
			"https://api.flow-like.com/api/v1/registry/package/x",
			"https://api.flow-like.com/api/v1/registry/package/x/versions",
			"https://example.com/widget-sandbox/x",
			"https://example.com/?next=/registry/package/x/widget-sandbox/1",
			"https://example.com/#/registry/package/x/widget-asset/1",
			"https://flow-widget.localhost.example.com/x",
			"https://localhost/x",
		]) {
			expect({ url, serving: isMicroWidgetServingUrl(url) }).toEqual({
				url,
				serving: false,
			});
		}
	});

	test("unparseable input fails closed", () => {
		expect(isMicroWidgetServingUrl("relative/without/base")).toBeTrue();
		expect(isMicroWidgetServingUrl("http://[::1")).toBeTrue();
	});
});

describe("legacy URL building", () => {
	test("desktop custom-protocol form", () => {
		expect(
			buildDesktopMicroWidgetSrc({
				packageId: "com.example.sales",
				bundleHash: "deadbeef",
				widgetId: "sales-chart",
				useHttpBridge: false,
			}),
		).toBe(
			"flow-widget://localhost/com.example.sales/deadbeef/frame/sales-chart",
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
			"http://flow-widget.localhost/com.example.sales/deadbeef/frame/sales-chart",
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
		).toBe("flow-widget://localhost/a%20b/h%231/frame/w%2F1");
	});

	test("web registry path", () => {
		expect(
			buildWebMicroWidgetPath("com.example.sales", "1.2.0", "sales-chart"),
		).toBe(
			"registry/package/com.example.sales/widget-asset/1.2.0/frame/sales-chart",
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

	test("contract event bucket allows 60 per second and refills after one second", () => {
		const bucket = new TokenBucket(
			MICRO_WIDGET_EVENT_BURST,
			MICRO_WIDGET_EVENT_RATE_PER_SECOND,
		);
		for (let i = 0; i < 60; i++) {
			expect(bucket.tryTake(1000)).toBeTrue();
		}
		expect(bucket.tryTake(1000)).toBeFalse();
		for (let i = 0; i < 60; i++) {
			expect(bucket.tryTake(2000)).toBeTrue();
		}
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

describe("legacy wrapper frame", () => {
	test("hosts that predate grants still receive the downloads query", () => {
		expect(
			buildDesktopMicroWidgetSrc({
				packageId: "com.example.sales",
				bundleHash: "deadbeef",
				widgetId: "sales-chart",
				useHttpBridge: false,
				allowDownloads: true,
			}),
		).toBe(
			"flow-widget://localhost/com.example.sales/deadbeef/frame/sales-chart?downloads=1",
		);
		expect(
			buildWebMicroWidgetPath(
				"com.example.sales",
				"1.2.0",
				"sales-chart",
				true,
			),
		).toBe(
			"registry/package/com.example.sales/widget-asset/1.2.0/frame/sales-chart?downloads=1",
		);
	});
});
