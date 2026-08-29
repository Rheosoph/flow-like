import { beforeEach, describe, expect, test } from "bun:test";
import type { IStorageState } from "../state/backend-state/storage-state";
import type { IStorageItemActionResult } from "../state/backend-state/types";
import {
	invalidateAssetUrl,
	isRootedPath,
	isStorageAssetPath,
	normalizeStorageAssetPath,
	peekAssetUrl,
	resetAssetUrlCache,
	resolveAssetUrl,
} from "./asset-url-cache";

const BUCKET = "https://bucket.s3.eu-central-1.amazonaws.com";

const compactUtc = (at: number) =>
	new Date(at)
		.toISOString()
		.replace(/[-:]/g, "")
		.replace(/\.\d{3}/, "");

/** A SigV4 link for `path`, signed now and good for `ttlSeconds`. */
function signed(path: string, ttlSeconds: number, signedAt = Date.now()) {
	return `${BUCKET}/${path}?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Date=${compactUtc(
		signedAt,
	)}&X-Amz-Expires=${ttlSeconds}&X-Amz-Signature=deadbeef`;
}

interface Recorder {
	readonly storageState: IStorageState;
	/** One entry per request, holding the prefixes it asked for. */
	readonly calls: string[][];
}

function recorder(
	respond: (prefix: string) => IStorageItemActionResult = (prefix) => ({
		prefix,
		url: signed(prefix, 3600),
	}),
): Recorder {
	const calls: string[][] = [];
	const storageState = {
		downloadStorageItems: async (_appId: string, prefixes: string[]) => {
			calls.push([...prefixes]);
			return prefixes.map(respond);
		},
	} as unknown as IStorageState;
	return { storageState, calls };
}

beforeEach(() => {
	resetAssetUrlCache();
});

describe("storage path classification", () => {
	test("app-relative paths are storage paths", () => {
		expect(isStorageAssetPath("media/logo.jpg")).toBe(true);
		expect(isStorageAssetPath("logo.jpg")).toBe(true);
		expect(isStorageAssetPath("storage://media/logo.jpg")).toBe(true);
	});

	test("anything that already addresses a resource is not", () => {
		for (const value of [
			"https://cdn.example.com/logo.png",
			"http://asset.localhost/C%3A%5Cusers",
			"data:image/png;base64,abc",
			"blob:https://app.example.com/1234",
			"asset://localhost/Users/me/logo.png",
			"file:///Users/me/logo.png",
		]) {
			expect(isStorageAssetPath(value)).toBe(false);
		}
	});

	test("rooted paths are never storage paths", () => {
		// `/images/logo.png` is the host's own asset, not an object in app storage.
		expect(isRootedPath("/images/logo.png")).toBe(true);
		expect(isRootedPath("C:\\Users\\me\\logo.png")).toBe(true);
		expect(isStorageAssetPath("/images/logo.png")).toBe(false);
		expect(isStorageAssetPath("C:\\Users\\me\\logo.png")).toBe(false);
	});

	test("empty and missing values are not storage paths", () => {
		expect(isStorageAssetPath("")).toBe(false);
		expect(isStorageAssetPath(undefined)).toBe(false);
		expect(isStorageAssetPath(null)).toBe(false);
	});

	test("the storage:// marker is stripped, and only as a prefix", () => {
		expect(normalizeStorageAssetPath("storage://media/logo.jpg")).toBe(
			"media/logo.jpg",
		);
		expect(normalizeStorageAssetPath("media/storage://logo.jpg")).toBe(
			"media/storage://logo.jpg",
		);
	});
});

describe("resolveAssetUrl", () => {
	test("signs a path and hands back the URL", async () => {
		const { storageState, calls } = recorder();

		const entry = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		expect(entry.resolved).toBe(true);
		expect(entry.url).toContain("/media/logo.jpg?");
		expect(calls).toEqual([["media/logo.jpg"]]);
	});

	test("the storage:// marker and the bare path share one entry", async () => {
		const { storageState, calls } = recorder();

		const marked = await resolveAssetUrl(
			"app",
			"storage://media/logo.jpg",
			storageState,
		);
		const bare = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		expect(bare.url).toBe(marked.url);
		expect(calls).toHaveLength(1);
	});

	test("reuses a live signature instead of signing again", async () => {
		const { storageState, calls } = recorder();

		const first = await resolveAssetUrl("app", "media/logo.jpg", storageState);
		const second = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		expect(second.url).toBe(first.url);
		expect(calls).toHaveLength(1);
	});

	test("the reuse window comes from the signature, not a fixed guess", async () => {
		const { storageState } = recorder((prefix) => ({
			prefix,
			url: signed(prefix, 3600),
		}));

		const entry = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		// Good for an hour, so reusable for all but its last five minutes.
		expect(entry.expiresAt - Date.now()).toBeGreaterThan(59 * 60 * 1000);
		expect(entry.expiresAt - entry.usableUntil).toBe(5 * 60 * 1000);
	});

	test("a short-lived signature is held for seconds, not a fixed window", async () => {
		const { storageState } = recorder((prefix) => ({
			prefix,
			url: signed(prefix, 120),
		}));

		const entry = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		// A credential this close to death cannot be replaced on every render, so
		// a floor applies — but the entry still lapses in seconds rather than
		// being trusted for the half hour a fixed TTL used to assume.
		const window = entry.usableUntil - Date.now();
		expect(window).toBeLessThanOrEqual(30_000);
		expect(window).toBeGreaterThan(25_000);
	});

	test("a URL carrying no signature never needs signing again", async () => {
		// What the desktop hands back for a local app: an address, not a credential.
		const { storageState, calls } = recorder((prefix) => ({
			prefix,
			url: `asset://localhost/Users/me/${prefix}`,
		}));

		const entry = await resolveAssetUrl("app", "logo.jpg", storageState);
		await resolveAssetUrl("app", "logo.jpg", storageState);

		expect(entry.expiresAt).toBe(Number.POSITIVE_INFINITY);
		expect(calls).toHaveLength(1);
	});

	test("concurrent asks for one page become a single request", async () => {
		const { storageState, calls } = recorder();

		const entries = await Promise.all([
			resolveAssetUrl("app", "a.jpg", storageState),
			resolveAssetUrl("app", "b.jpg", storageState),
			resolveAssetUrl("app", "a.jpg", storageState),
			resolveAssetUrl("app", "c.jpg", storageState),
		]);

		expect(calls).toEqual([["a.jpg", "b.jpg", "c.jpg"]]);
		expect(entries[0].url).toBe(entries[2].url);
	});

	test("apps are signed separately", async () => {
		const { storageState, calls } = recorder();

		await Promise.all([
			resolveAssetUrl("app-one", "logo.jpg", storageState),
			resolveAssetUrl("app-two", "logo.jpg", storageState),
		]);

		expect(calls).toHaveLength(2);
	});

	test("a path that cannot be signed falls back to the path itself", async () => {
		const { storageState } = recorder((prefix) => ({
			prefix,
			error: "not found",
		}));

		const entry = await resolveAssetUrl("app", "media/gone.jpg", storageState);

		expect(entry.resolved).toBe(false);
		expect(entry.url).toBe("media/gone.jpg");
	});

	test("a failed request does not reject its callers", async () => {
		const storageState = {
			downloadStorageItems: async () => {
				throw new Error("offline");
			},
		} as unknown as IStorageState;

		const entry = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		expect(entry.resolved).toBe(false);
		expect(entry.url).toBe("media/logo.jpg");
	});
});

describe("peekAssetUrl", () => {
	test("is empty until a path has been resolved", async () => {
		const { storageState } = recorder();

		expect(peekAssetUrl("app", "media/logo.jpg")).toBeUndefined();
		const entry = await resolveAssetUrl("app", "media/logo.jpg", storageState);

		expect(peekAssetUrl("app", "media/logo.jpg")?.url).toBe(entry.url);
		expect(peekAssetUrl("app", "storage://media/logo.jpg")?.url).toBe(
			entry.url,
		);
	});

	test("does not answer for another app", async () => {
		const { storageState } = recorder();
		await resolveAssetUrl("app-one", "media/logo.jpg", storageState);

		expect(peekAssetUrl("app-two", "media/logo.jpg")).toBeUndefined();
	});
});

describe("invalidateAssetUrl", () => {
	test("refuses a path that was only just signed", async () => {
		const { storageState, calls } = recorder();
		await resolveAssetUrl("app", "media/logo.jpg", storageState);

		// A link that fails for a reason signing cannot fix — the object is gone,
		// the caller lost access — must not re-sign on every failed load.
		expect(invalidateAssetUrl("app", "media/logo.jpg")).toBe(false);
		await resolveAssetUrl("app", "media/logo.jpg", storageState);
		expect(calls).toHaveLength(1);
	});

	test("ignores values it was never asked to sign", () => {
		expect(invalidateAssetUrl("app", undefined)).toBe(false);
		expect(invalidateAssetUrl(undefined, "media/logo.jpg")).toBe(false);
	});
});
