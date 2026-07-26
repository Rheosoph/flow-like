import { beforeEach, describe, expect, test } from "bun:test";
import {
	resetStableAssetUrls,
	stabilizeMetadata,
	stabilizeSignedUrls,
	stableAssetUrl,
} from "./stable-asset-url";

const OBJECT =
	"https://bucket.s3.eu-central-1.amazonaws.com/media/apps/a/i.webp";

/** Two signatures of the same object, minted a minute apart. */
const signedAws = (date: string, ttl = 86400) =>
	`${OBJECT}?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Date=${date}&X-Amz-Expires=${ttl}&X-Amz-Signature=${date}beef`;

const isoNow = (offsetMs = 0) =>
	new Date(Date.now() + offsetMs)
		.toISOString()
		.replace(/[-:]/g, "")
		.replace(/\.\d{3}/, "");

beforeEach(() => {
	resetStableAssetUrls();
});

describe("stableAssetUrl", () => {
	test("collapses a re-signed URL onto the first one seen", () => {
		const first = signedAws(isoNow());
		const second = signedAws(isoNow(60_000));

		expect(stableAssetUrl(first)).toBe(first);
		expect(stableAssetUrl(second)).toBe(first);
	});

	test("keeps distinct objects apart", () => {
		const icon = signedAws(isoNow());
		const other = `https://bucket.s3.eu-central-1.amazonaws.com/media/apps/b/i.webp?X-Amz-Date=${isoNow()}&X-Amz-Expires=86400`;

		expect(stableAssetUrl(icon)).toBe(icon);
		expect(stableAssetUrl(other)).toBe(other);
	});

	test("adopts a new signature once the stored one is near expiry", () => {
		// Signed 23h59m ago with a 24h lifetime: inside the safety margin.
		const stale = signedAws(isoNow(-((24 * 60 - 1) * 60 * 1000)));
		const fresh = signedAws(isoNow());

		expect(stableAssetUrl(stale)).toBe(stale);
		expect(stableAssetUrl(fresh)).toBe(fresh);
	});

	test("passes through URLs that carry no signature", () => {
		const plain = "https://cdn.example.com/logo.webp";
		const asset = "asset://localhost/home/user/icon.webp";
		const relative = "/app-logo.webp";

		expect(stableAssetUrl(plain)).toBe(plain);
		expect(stableAssetUrl(asset)).toBe(asset);
		expect(stableAssetUrl(relative)).toBe(relative);
	});

	test("passes nullish values through untouched", () => {
		expect(stableAssetUrl(undefined)).toBeUndefined();
		expect(stableAssetUrl(null)).toBeNull();
		expect(stableAssetUrl("")).toBe("");
	});

	test("understands Google and Azure signatures", () => {
		const gcsFirst = `https://storage.googleapis.com/b/i.webp?X-Goog-Date=${isoNow()}&X-Goog-Expires=86400&X-Goog-Signature=aa`;
		const gcsSecond = `https://storage.googleapis.com/b/i.webp?X-Goog-Date=${isoNow(60_000)}&X-Goog-Expires=86400&X-Goog-Signature=bb`;
		expect(stableAssetUrl(gcsFirst)).toBe(gcsFirst);
		expect(stableAssetUrl(gcsSecond)).toBe(gcsFirst);

		const expiry = new Date(Date.now() + 86_400_000).toISOString();
		const azFirst = `https://acct.blob.core.windows.net/c/i.webp?se=${expiry}&sig=aa`;
		const azSecond = `https://acct.blob.core.windows.net/c/i.webp?se=${expiry}&sig=bb`;
		expect(stableAssetUrl(azFirst)).toBe(azFirst);
		expect(stableAssetUrl(azSecond)).toBe(azFirst);
	});
});

describe("stabilizeMetadata", () => {
	test("rewrites media fields onto the remembered URLs", () => {
		const first = signedAws(isoNow());
		stableAssetUrl(first);

		const result = stabilizeMetadata({
			name: "App",
			icon: signedAws(isoNow(60_000)),
			thumbnail: null,
			preview_media: [],
		});

		expect(result.icon).toBe(first);
		expect(result.name).toBe("App");
	});

	test("returns the same object when nothing changed", () => {
		const metadata = { icon: "/app-logo.webp", thumbnail: null };
		expect(stabilizeMetadata(metadata)).toBe(metadata);
	});

	test("stabilizes preview media entries", () => {
		const first = signedAws(isoNow());
		stableAssetUrl(first);

		const result = stabilizeMetadata({
			preview_media: [signedAws(isoNow(60_000))],
		});

		expect(result.preview_media).toEqual([first]);
	});

	test("passes undefined through", () => {
		expect(stabilizeMetadata(undefined)).toBeUndefined();
	});
});

describe("stabilizeSignedUrls", () => {
	test("rewrites url fields and leaves untouched entries by reference", () => {
		const first = signedAws(isoNow());
		stableAssetUrl(first);

		const unchanged = { prefix: "b", error: "nope" };
		const [rewritten, passthrough] = stabilizeSignedUrls([
			{ prefix: "a", url: signedAws(isoNow(60_000)) },
			unchanged,
		]);

		expect(rewritten.url).toBe(first);
		expect(passthrough).toBe(unchanged);
	});
});
