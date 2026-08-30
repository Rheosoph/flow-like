import { beforeEach, describe, expect, test } from "bun:test";
import {
	confirmStableAssetUrl,
	hasExpiredAssetUrl,
	isExpiredAssetUrl,
	mergeMetadataMedia,
	recoverStableAssetUrl,
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

	test("does not collapse versioned or transformed variants of one path", () => {
		const first = `${signedAws(isoNow())}&versionId=v1&width=128`;
		const otherVersion = `${signedAws(isoNow(60_000))}&versionId=v2&width=128`;
		const otherSize = `${signedAws(isoNow(120_000))}&versionId=v1&width=512`;

		expect(stableAssetUrl(first)).toBe(first);
		expect(stableAssetUrl(otherVersion)).toBe(otherVersion);
		expect(stableAssetUrl(otherSize)).toBe(otherSize);
	});

	test("does not reuse a URL across signing credentials", () => {
		const first = `${signedAws(isoNow())}&X-Amz-Credential=old-key`;
		const rotated = `${signedAws(isoNow(60_000))}&X-Amz-Credential=new-key`;

		expect(stableAssetUrl(first)).toBe(first);
		expect(stableAssetUrl(rotated)).toBe(rotated);
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

	test("evicts a failed URL and promotes the newest signature", () => {
		const bad = signedAws(isoNow());
		const fresh = signedAws(isoNow(60_000));

		expect(stableAssetUrl(bad)).toBe(bad);
		expect(stableAssetUrl(fresh)).toBe(bad);
		expect(recoverStableAssetUrl(bad)).toBe(fresh);
		expect(stableAssetUrl(fresh)).toBe(fresh);
	});

	test("drops a failed URL when no replacement has been observed", () => {
		const bad = signedAws(isoNow());
		expect(stableAssetUrl(bad)).toBe(bad);
		expect(recoverStableAssetUrl(bad)).toBeUndefined();

		const fresh = signedAws(isoNow(60_000));
		expect(stableAssetUrl(fresh)).toBe(fresh);
	});

	test("accepts successful-load confirmation", () => {
		const first = signedAws(isoNow());
		expect(stableAssetUrl(first)).toBe(first);
		expect(() => confirmStableAssetUrl(first)).not.toThrow();
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

describe("isExpiredAssetUrl", () => {
	test("reports a signature whose deadline has passed", () => {
		// Signed 25h ago with a 24h lifetime.
		expect(isExpiredAssetUrl(signedAws(isoNow(-25 * 60 * 60 * 1000)))).toBe(
			true,
		);
	});

	test("leaves live and unsigned URLs alone", () => {
		expect(isExpiredAssetUrl(signedAws(isoNow()))).toBe(false);
		expect(isExpiredAssetUrl("asset://localhost/home/user/icon.webp")).toBe(
			false,
		);
		expect(isExpiredAssetUrl("/app-logo.webp")).toBe(false);
		expect(isExpiredAssetUrl(undefined)).toBe(false);
		expect(isExpiredAssetUrl(null)).toBe(false);
	});
});

describe("mergeMetadataMedia", () => {
	test("takes fresh media over a cached signature", () => {
		const cached = {
			name: "App",
			icon: signedAws(isoNow(-25 * 60 * 60 * 1000)),
			thumbnail: signedAws(isoNow(-25 * 60 * 60 * 1000)),
		};
		const fresh = {
			name: "Renamed",
			icon: signedAws(isoNow()),
			thumbnail: signedAws(isoNow()),
		};

		const merged = mergeMetadataMedia(cached, fresh);

		expect(merged.icon).toBe(fresh.icon);
		expect(merged.thumbnail).toBe(fresh.thumbnail);
		expect(merged.name).toBe("App");
	});

	test("fills media the cached record never had", () => {
		// The shape a cloud-hosted app has locally: a record with no artwork.
		const cached: { name: string; icon?: string | null } = { name: "App" };
		const fresh = { icon: signedAws(isoNow()), thumbnail: null };

		expect(mergeMetadataMedia(cached, fresh).icon).toBe(fresh.icon);
	});

	test("keeps device-local artwork the browser already holds", () => {
		const cached = {
			icon: "asset://localhost/home/user/icon.webp",
			thumbnail: "data:image/webp;base64,AA",
		};

		const merged = mergeMetadataMedia(cached, {
			icon: signedAws(isoNow()),
			thumbnail: signedAws(isoNow()),
		});

		expect(merged).toBe(cached);
	});

	test("keeps a cached gallery only while every entry is durable", () => {
		const durable = ["asset://localhost/a.webp", "/b.webp"];
		const fresh = { preview_media: [signedAws(isoNow())] };

		expect(
			mergeMetadataMedia({ preview_media: durable }, fresh).preview_media,
		).toBe(durable);
		expect(
			mergeMetadataMedia(
				{ preview_media: [...durable, signedAws(isoNow(-60_000))] },
				fresh,
			).preview_media,
		).toBe(fresh.preview_media);
	});

	test("keeps the cached record when there is nothing to merge from", () => {
		const cached = { icon: signedAws(isoNow(-60_000)) };
		expect(mergeMetadataMedia(cached, undefined)).toBe(cached);
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

describe("hasExpiredAssetUrl", () => {
	/** What a cached page surface looks like once its links have died. */
	const surfaceWith = (url: string) => ({
		id: "page",
		rootComponentId: "root",
		components: {
			root: { id: "root", component: { type: "column", children: ["img"] } },
			img: {
				id: "img",
				component: { type: "image", src: { literalString: url } },
			},
		},
	});

	test("finds a dead signature nested anywhere in a record", () => {
		const dead = signedAws(isoNow(-2 * 86_400_000));
		expect(isExpiredAssetUrl(dead)).toBe(true);
		expect(hasExpiredAssetUrl(surfaceWith(dead))).toBe(true);
	});

	test("passes a record whose signatures are all still live", () => {
		expect(hasExpiredAssetUrl(surfaceWith(signedAws(isoNow())))).toBe(false);
	});

	test("ignores strings that are not signed URLs", () => {
		expect(
			hasExpiredAssetUrl({
				text: "X-Amz-Date=20200101T000000Z",
				path: "media/apps/a/i.webp",
				n: 3,
				missing: null,
			}),
		).toBe(false);
	});
});
