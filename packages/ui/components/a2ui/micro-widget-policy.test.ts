import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../lib/api-error";
import {
	WIDGET_SOURCE_LEVELS,
	type WidgetPolicy,
	WidgetPolicyChangedError,
	WidgetRuntimeSourcesError,
	canonicalWidgetRuntimeRequest,
	capabilityPolicy,
	isDesktopWidgetGrant,
	isEmptyPolicy,
	isPolicyChangedError,
	isWebWidgetGrant,
	isWidgetGrantUnavailableError,
	isWidgetRuntimeComponent,
	isWidgetRuntimeDescribeUnsupportedError,
	maxWidgetSourceLevel,
	normalizeWidgetPolicy,
	parseWidgetGrantResponse,
	parseWidgetInputPath,
	parseWidgetPolicyDescriptor,
	policyCapabilities,
	policyCovers,
	policyHostCount,
	policyHosts,
	readWidgetNetwork,
	readWidgetPolicy,
	readWidgetSourceLevel,
	splitWidgetPolicyDescriptor,
	widgetRuntimeRequestKey,
	widgetRuntimeRequestSources,
	widgetRuntimeSourcesErrorCode,
	widgetSourceLevelCovers,
	widgetSourceLevelRank,
} from "./micro-widget-policy";

const DIGEST = `sha256:${"ab".repeat(32)}`;
const HASH = "cd".repeat(32);

const network: WidgetPolicy = {
	workers: true,
	csp: {
		connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
		imgSrc: ["https://a.tile.openstreetmap.org"],
		fontSrc: ["https://live.example.com"],
	},
};

const descriptor = (overrides: Record<string, unknown> = {}) => ({
	source: "registry:hub.flow-like.com",
	packageId: "com.example.maps",
	bundleHash: HASH,
	widgetId: "live-map",
	preview: false,
	status: "ok",
	policy: network,
	policyDigest: DIGEST,
	...overrides,
});

describe("policy helpers", () => {
	test("capabilities count only own true flags", () => {
		const inherited = Object.create({ wasm: true }) as WidgetPolicy;
		inherited.workers = true;
		(inherited as Record<string, unknown>).media = "true";
		expect(policyCapabilities(inherited)).toEqual(["workers"]);
		expect(policyCapabilities(null)).toEqual([]);
	});

	test("hosts are distinct across directives and schemes", () => {
		expect(policyHosts(network)).toEqual([
			"a.tile.openstreetmap.org",
			"api.maptiler.com",
			"live.example.com",
		]);
		expect(policyHostCount(network)).toBe(3);
		expect(policyHostCount({ workers: true })).toBe(0);
	});

	test("empty means no capability and no source", () => {
		expect(isEmptyPolicy({})).toBe(true);
		expect(isEmptyPolicy({ workers: false, csp: { imgSrc: [] } })).toBe(true);
		expect(isEmptyPolicy({ downloads: true })).toBe(false);
		expect(
			isEmptyPolicy({ csp: { styleSrc: ["https://x.example.org"] } }),
		).toBe(false);
	});

	test("coverage is per capability and per directive source", () => {
		expect(policyCovers(network, network)).toBe(true);
		expect(policyCovers(network, {})).toBe(true);
		expect(policyCovers(null, {})).toBe(false);
		expect(policyCovers(network, { wasm: true })).toBe(false);
		expect(
			policyCovers(network, { csp: { imgSrc: ["https://api.maptiler.com"] } }),
		).toBe(false);
		expect(
			policyCovers(
				{ csp: { connectSrc: ["https://api.maptiler.com"] } },
				{ csp: { connectSrc: ["wss://api.maptiler.com"] } },
			),
		).toBe(false);
	});

	test("normalization keeps true flags and sorted unique sources", () => {
		expect(
			normalizeWidgetPolicy({
				workers: true,
				wasm: false,
				csp: {
					connectSrc: [
						"wss://b.example.com",
						"https://a.example.com",
						"wss://b.example.com",
					],
					imgSrc: [],
				},
			}),
		).toEqual({
			workers: true,
			csp: { connectSrc: ["https://a.example.com", "wss://b.example.com"] },
		});
		expect(capabilityPolicy(["wasm", "downloads"])).toEqual({
			downloads: true,
			wasm: true,
		});
	});

	test("reading rejects fields the host does not understand", () => {
		expect(readWidgetPolicy(network)).toEqual(network);
		expect(readWidgetPolicy({ workers: false, csp: {} })).toEqual({});
		expect(readWidgetPolicy({ frames: true })).toBeNull();
		expect(readWidgetPolicy({ workers: "yes" })).toBeNull();
		expect(
			readWidgetPolicy({ csp: { frameSrc: ["https://x.example.org"] } }),
		).toBeNull();
		expect(
			readWidgetPolicy({ csp: { imgSrc: "https://x.example.org" } }),
		).toBeNull();
		expect(readWidgetPolicy({ csp: { imgSrc: [""] } })).toBeNull();
		expect(readWidgetPolicy([])).toBeNull();
	});
});

describe("parseWidgetPolicyDescriptor", () => {
	test("accepts the pinned descriptor shape", () => {
		expect(
			parseWidgetPolicyDescriptor(
				descriptor({ packageVersion: "1.2.0", invalidReason: undefined }),
				{ packageId: "com.example.maps", widgetId: "live-map", preview: false },
			),
		).toEqual({
			...descriptor(),
			packageVersion: "1.2.0",
			networkInputs: [],
		} as never);
		const invalid = parseWidgetPolicyDescriptor(
			descriptor({
				status: "invalid",
				invalidReason: "reserved host",
				policy: {},
			}),
		);
		expect(invalid.status).toBe("invalid");
		expect(invalid.invalidReason).toBe("reserved host");
	});

	test("rejects malformed or unsupported responses", () => {
		for (const [value, message] of [
			[null, /not an object/],
			[descriptor({ status: "maybe" }), /"status"/],
			[descriptor({ preview: "false" }), /"preview"/],
			[descriptor({ policyDigest: "sha256:xyz" }), /"policyDigest"/],
			[descriptor({ source: "" }), /"source"/],
			[
				descriptor({
					policy: { csp: { scriptSrc: ["https://x.example.org"] } },
				}),
				/unsupported "policy"/,
			],
		] as const) {
			expect(() => parseWidgetPolicyDescriptor(value)).toThrow(message);
		}
	});

	test("rejects an answer about another widget", () => {
		expect(() =>
			parseWidgetPolicyDescriptor(descriptor(), { widgetId: "other" }),
		).toThrow(
			/com\.example\.maps\/live-map answers for widgetId "live-map" instead of "other"/,
		);
		expect(() =>
			parseWidgetPolicyDescriptor(descriptor(), {
				bundleHash: "ef".repeat(32),
			}),
		).toThrow(/bundleHash/);
		expect(() =>
			parseWidgetPolicyDescriptor(descriptor(), { preview: true }),
		).toThrow(/preview/);
		expect(() =>
			parseWidgetPolicyDescriptor(descriptor(), { packageVersion: "1.2.0" }),
		).toThrow(/packageVersion/);
	});
});

describe("grants", () => {
	test("desktop ids are 64 lowercase hex and web grants are compact JWTs", () => {
		expect(isDesktopWidgetGrant("0f".repeat(32))).toBe(true);
		expect(isDesktopWidgetGrant("0F".repeat(32))).toBe(false);
		expect(isDesktopWidgetGrant("0f".repeat(31))).toBe(false);
		expect(isWebWidgetGrant("a-_.b.c")).toBe(true);
		expect(isWebWidgetGrant("a.b")).toBe(false);
		expect(isWebWidgetGrant("a..c")).toBe(false);
		expect(isWebWidgetGrant("a.b.c=")).toBe(false);
		expect(isWebWidgetGrant(`${"a".repeat(2044)}.b.c`)).toBe(true);
		expect(isWebWidgetGrant(`${"a".repeat(2045)}.b.c`)).toBe(false);
	});

	test("grant responses are validated", () => {
		expect(
			parseWidgetGrantResponse(
				{ grant: null, expiresIn: 0, policyDigest: DIGEST },
				isDesktopWidgetGrant,
			),
		).toEqual({
			grant: null,
			expiresIn: 0,
			policyDigest: DIGEST,
			runtime: null,
		});
		expect(
			parseWidgetGrantResponse(
				{ grant: "a.b.c", expiresIn: 3600, policyDigest: DIGEST },
				isWebWidgetGrant,
			).grant,
		).toBe("a.b.c");
		expect(() =>
			parseWidgetGrantResponse(
				{ grant: "a.b.c", expiresIn: 3600, policyDigest: DIGEST },
				isDesktopWidgetGrant,
			),
		).toThrow(/malformed grant/);
		expect(() =>
			parseWidgetGrantResponse(
				{ grant: null, expiresIn: -1, policyDigest: DIGEST },
				isWebWidgetGrant,
			),
		).toThrow(/expiresIn/);
		expect(() =>
			parseWidgetGrantResponse(
				{ grant: null, expiresIn: 1, policyDigest: "nope" },
				isWebWidgetGrant,
			),
		).toThrow(/policyDigest/);
	});

	test("policy_changed is recognized from every backend", () => {
		expect(isPolicyChangedError(new WidgetPolicyChangedError())).toBe(true);
		expect(isPolicyChangedError("policy_changed: digest mismatch")).toBe(true);
		expect(
			isPolicyChangedError({ error: "policy_changed: expected sha256:…" }),
		).toBe(true);
		expect(isPolicyChangedError(new Error("policy_changed"))).toBe(true);
		expect(
			isPolicyChangedError(
				new ApiResponseError({ status: 409, message: "Policy changed" }),
			),
		).toBe(true);
		expect(isPolicyChangedError("widget not installed")).toBe(false);
		expect(
			isPolicyChangedError(
				new ApiResponseError({ status: 404, message: "policy_changed" }),
			),
		).toBe(false);
		expect(isPolicyChangedError(undefined)).toBe(false);
	});

	test("web grant responses carry the runtime component of their mint", () => {
		const token = "a.b.c";
		expect(
			parseWidgetGrantResponse(
				{
					grant: token,
					expiresIn: 1,
					policyDigest: DIGEST,
					runtime: "eyJ4IjpbXX0",
				},
				isWebWidgetGrant,
			).runtime,
		).toBe("eyJ4IjpbXX0");
		expect(
			parseWidgetGrantResponse(
				{ grant: token, expiresIn: 1, policyDigest: DIGEST },
				isWebWidgetGrant,
			).runtime,
		).toBeNull();
		for (const runtime of ["a~b", "", "abcde", "a".repeat(1367), 5]) {
			expect(() =>
				parseWidgetGrantResponse(
					{ grant: token, expiresIn: 1, policyDigest: DIGEST, runtime },
					isWebWidgetGrant,
				),
			).toThrow(/runtime component/);
		}
		expect(() =>
			parseWidgetGrantResponse(
				{ grant: null, expiresIn: 1, policyDigest: DIGEST, runtime: "abcd" },
				isWebWidgetGrant,
			),
		).toThrow(/without a grant/);
		expect(isWidgetRuntimeComponent("a".repeat(1366))).toBe(true);
		expect(isWidgetRuntimeComponent("ab=")).toBe(false);
	});

	test("refused runtime requests are recognized from every backend", () => {
		expect(
			widgetRuntimeSourcesErrorCode(
				"invalid_runtime_sources: slot tileUrl appears twice",
			),
		).toBe("INVALID_RUNTIME_SOURCES");
		expect(
			widgetRuntimeSourcesErrorCode({
				error: "runtime_sources_in_preview: previews have no runtime sources",
			}),
		).toBe("RUNTIME_SOURCES_IN_PREVIEW");
		expect(
			widgetRuntimeSourcesErrorCode(
				new ApiResponseError({
					status: 400,
					code: "INVALID_RUNTIME_SOURCES",
					message: "too many",
				}),
			),
		).toBe("INVALID_RUNTIME_SOURCES");
		expect(
			widgetRuntimeSourcesErrorCode(
				new WidgetRuntimeSourcesError("RUNTIME_SOURCES_IN_PREVIEW", "x"),
			),
		).toBe("RUNTIME_SOURCES_IN_PREVIEW");
		expect(widgetRuntimeSourcesErrorCode("policy_changed: x")).toBeNull();
		expect(
			isWidgetRuntimeDescribeUnsupportedError(
				new ApiResponseError({ status: 405, message: "Method Not Allowed" }),
			),
		).toBe(true);
		expect(
			isWidgetRuntimeDescribeUnsupportedError(
				new ApiResponseError({ status: 400, message: "bad" }),
			),
		).toBe(false);
	});

	test("a web API without a signing key is recognized", () => {
		expect(
			isWidgetGrantUnavailableError(
				new ApiResponseError({ status: 503, message: "unavailable" }),
			),
		).toBe(true);
		expect(
			isWidgetGrantUnavailableError(
				new ApiResponseError({ status: 409, message: "changed" }),
			),
		).toBe(false);
		expect(isWidgetGrantUnavailableError("503")).toBe(false);
	});
});

const RUNTIME_HOST = "https://a.tiles.customer-maps.com";
const WILDCARD = "https://*.s3.eu-central-1.amazonaws.com";

const effective: WidgetPolicy = {
	workers: true,
	csp: {
		connectSrc: ["https://api.cesium.com", RUNTIME_HOST, WILDCARD],
		imgSrc: [RUNTIME_HOST, WILDCARD],
	},
};

const networkSource = (overrides: Record<string, unknown>) => ({
	source: "https://api.cesium.com",
	directives: ["connectSrc"],
	origin: "declared",
	kind: "service",
	level: "external",
	host: "api.cesium.com",
	emphasis: "cesium.com",
	providerId: "cesium-ion",
	provider: "Cesium ion",
	aboutKey: "widgetSourceAboutMapData",
	...overrides,
});

const classified = (purposes?: unknown[]) => ({
	level: "broad",
	catalogVersion: 1,
	pslVersion: "2026-09-15_10-18-26_UTC",
	stale: false,
	purposes: purposes ?? [
		{
			reason: "Loads globe terrain, imagery and 3D tiles from Cesium ion",
			level: "external",
			sources: [networkSource({})],
		},
		{
			reason: "Loads map layers from Amazon S3 buckets in Frankfurt",
			level: "broad",
			sources: [
				networkSource({
					source: WILDCARD,
					directives: ["connectSrc", "imgSrc"],
					kind: "shared-wildcard",
					level: "broad",
					host: "*.s3.eu-central-1.amazonaws.com",
					emphasis: "s3.eu-central-1.amazonaws.com",
					providerId: "aws-s3",
					provider: "Amazon S3",
					aboutKey: "widgetSourceAboutObjectStorage",
				}),
			],
		},
		{
			reason: "Loads map tiles from tile servers given to it at runtime",
			level: "external",
			inputs: ["tileUrl"],
			sources: [
				networkSource({
					source: RUNTIME_HOST,
					directives: ["connectSrc", "imgSrc"],
					origin: "runtime",
					slot: "tileUrl",
					kind: "exact",
					host: "a.tiles.customer-maps.com",
					emphasis: "customer-maps.com",
					providerId: undefined,
					provider: undefined,
					aboutKey: undefined,
				}),
			],
		},
	],
});

const runtimeDescriptor = (overrides: Record<string, unknown> = {}) =>
	descriptor({
		source: "hub",
		packageVersion: "1.4.0",
		policy: effective,
		networkInputs: [
			{
				path: "tileUrl",
				purpose: 2,
				directives: ["connectSrc", "imgSrc"],
				template: { subdomainsInput: "tileSubdomains" },
			},
		],
		platformStorage: [
			{
				origin: "https://flow-like-content.s3.eu-central-1.amazonaws.com",
				pathPrefix: "/apps/",
			},
		],
		runtime: {
			status: "ok",
			declaredDigest: DIGEST,
			runtimeDigest: `sha256:${"cd".repeat(32)}`,
			rejected: [
				{
					slot: "tileUrl",
					source: "https://10-0-0-1.sslip.io",
					code: "reserved-name",
				},
			],
		},
		engine: { wildcardSources: true, runtimeSources: true, localMedia: true },
		network: classified(),
		futureField: { ignored: true },
		...overrides,
	});

describe("levels", () => {
	test("levels are ordered and unknown levels count as the worst", () => {
		expect(WIDGET_SOURCE_LEVELS.map(widgetSourceLevelRank)).toEqual([
			0, 1, 2, 3,
		]);
		expect(readWidgetSourceLevel("shared")).toBe("shared");
		expect(readWidgetSourceLevel("catastrophic")).toBe("broad");
		expect(maxWidgetSourceLevel(["known", "shared", "external"])).toBe(
			"shared",
		);
		expect(maxWidgetSourceLevel([])).toBeNull();
		expect(widgetSourceLevelCovers("shared", "external")).toBe(true);
		expect(widgetSourceLevelCovers("external", "shared")).toBe(false);
		expect(widgetSourceLevelCovers(undefined, "known")).toBe(false);
	});
});

describe("runtime descriptor fields", () => {
	test("the pinned descriptor parses with every addition", () => {
		const parsed = parseWidgetPolicyDescriptor(runtimeDescriptor());
		expect(parsed.networkInputs).toEqual([
			{
				path: "tileUrl",
				purpose: 2,
				directives: ["connectSrc", "imgSrc"],
				template: { subdomainsInput: "tileSubdomains" },
			},
		]);
		expect(parsed.platformStorage).toEqual([
			{
				origin: "https://flow-like-content.s3.eu-central-1.amazonaws.com",
				pathPrefix: "/apps/",
			},
		]);
		expect(parsed.runtime).toEqual({
			status: "ok",
			declaredDigest: DIGEST,
			runtimeDigest: `sha256:${"cd".repeat(32)}`,
			rejected: [
				{
					slot: "tileUrl",
					source: "https://10-0-0-1.sslip.io",
					code: "reserved-name",
				},
			],
		});
		expect(parsed.engine).toEqual({
			wildcardSources: true,
			runtimeSources: true,
			localMedia: true,
		});
		expect(parsed.network?.level).toBe("broad");
		expect(parsed.network?.purposes.map((purpose) => purpose.level)).toEqual([
			"external",
			"broad",
			"external",
		]);
		expect(parsed.network?.purposes[2].inputs).toEqual(["tileUrl"]);
		expect("futureField" in parsed).toBe(false);
	});

	test("enforcement fields are parsed strictly", () => {
		const bad = (overrides: Record<string, unknown>, message: RegExp) =>
			expect(() =>
				parseWidgetPolicyDescriptor(runtimeDescriptor(overrides)),
			).toThrow(message);
		bad(
			{
				networkInputs: [
					{ path: "tile url", purpose: 0, directives: ["imgSrc"] },
				],
			},
			/network input path/,
		);
		bad(
			{
				networkInputs: [
					{ path: "a.b.c.d.e.f.g", purpose: 0, directives: ["imgSrc"] },
				],
			},
			/network input path/,
		);
		bad(
			{
				networkInputs: [
					{ path: "tileUrl", purpose: -1, directives: ["imgSrc"] },
				],
			},
			/purpose/,
		);
		bad(
			{ networkInputs: [{ path: "tileUrl", purpose: 0, directives: [] }] },
			/directives/,
		);
		bad(
			{
				networkInputs: [
					{ path: "tileUrl", purpose: 0, directives: ["frameSrc"] },
				],
			},
			/directives/,
		);
		bad(
			{
				networkInputs: [
					{ path: "tileUrl", purpose: 0, directives: ["imgSrc"], extra: 1 },
				],
			},
			/unknown field "extra"/,
		);
		bad(
			{
				networkInputs: [
					{
						path: "tileUrl",
						purpose: 0,
						directives: ["imgSrc"],
						template: { subdomains: ["A"] },
					},
				],
			},
			/template subdomains/,
		);
		bad(
			{
				networkInputs: [
					{
						path: "tileUrl",
						purpose: 0,
						directives: ["imgSrc"],
						template: { wildcard: true },
					},
				],
			},
			/unknown field "wildcard"/,
		);
		bad(
			{
				networkInputs: [
					{ path: "a", purpose: 0, directives: ["imgSrc"] },
					{ path: "a", purpose: 1, directives: ["imgSrc"] },
				],
			},
			/network input path/,
		);
		bad(
			{
				platformStorage: [
					{ origin: "https://x.example.com/path", pathPrefix: "/apps/" },
				],
			},
			/platform storage origin/,
		);
		bad(
			{
				platformStorage: [
					{ origin: "https://x.example.com", pathPrefix: "apps" },
				],
			},
			/path prefix/,
		);
		bad(
			{ runtime: { status: "maybe", declaredDigest: DIGEST } },
			/runtime.status/,
		);
		bad(
			{ runtime: { status: "ok", declaredDigest: "sha256:nope" } },
			/declaredDigest/,
		);
		bad(
			{
				runtime: {
					status: "none",
					declaredDigest: DIGEST,
					runtimeDigest: DIGEST,
				},
			},
			/without ok/,
		);
		bad(
			{
				runtime: {
					status: "ok",
					declaredDigest: DIGEST,
					rejected: [{ slot: "a", source: "b" }],
				},
			},
			/rejected/,
		);
	});

	test("display fields are parsed leniently", () => {
		expect(
			parseWidgetPolicyDescriptor(
				runtimeDescriptor({ engine: { localMedia: "no" } }),
			).engine,
		).toBeUndefined();
		expect(
			parseWidgetPolicyDescriptor(runtimeDescriptor({ network: "classified" }))
				.network,
		).toBeUndefined();
	});

	test("invalid and preview descriptors carry no runtime facts", () => {
		for (const overrides of [
			{ status: "invalid", policy: {} },
			{ preview: true, policy: {} },
		]) {
			const parsed = parseWidgetPolicyDescriptor(
				runtimeDescriptor({ ...overrides, network: undefined }),
			);
			expect(parsed.networkInputs).toEqual([]);
			expect(parsed.platformStorage).toBeUndefined();
			expect(parsed.runtime).toBeUndefined();
			expect(parsed.engine).toBeUndefined();
			expect(parsed.network).toBeUndefined();
		}
	});
});

describe("readWidgetNetwork", () => {
	const purposesOf = (source: Record<string, unknown>) =>
		classified([
			{
				reason: "Loads things",
				level: "known",
				sources: [networkSource(source)],
			},
		]);
	const only: WidgetPolicy = {
		csp: { connectSrc: ["https://api.cesium.com"] },
	};

	test("unknown levels are broad and unknown kinds are kept", () => {
		const read = readWidgetNetwork(
			purposesOf({ level: "galactic", kind: "satellite-uplink" }),
			only,
		);
		expect(read?.purposes[0].sources[0]).toMatchObject({
			level: "broad",
			kind: "satellite-uplink",
		});
		expect(read?.purposes[0].level).toBe("broad");
		expect(read?.level).toBe("broad");
	});

	test("emphasis must end the host, providers stay short and about keys stay known", () => {
		const source = readWidgetNetwork(
			purposesOf({
				emphasis: "evil.com",
				provider: "P".repeat(41),
				aboutKey: "widgetSourceAboutAnything",
			}),
			only,
		)?.purposes[0].sources[0];
		expect(source?.emphasis).toBe("api.cesium.com");
		expect(source?.provider).toBeUndefined();
		expect(source?.aboutKey).toBeUndefined();
		expect(
			readWidgetNetwork(purposesOf({ emphasis: "ium.com" }), only)?.purposes[0]
				.sources[0].emphasis,
		).toBe("api.cesium.com");
	});

	test("sources outside the policy are dropped and unclassified policy sources void the object", () => {
		const read = readWidgetNetwork(
			classified([
				{
					reason: "Loads things",
					level: "known",
					sources: [
						networkSource({}),
						networkSource({ source: "https://elsewhere.example.com" }),
						networkSource({ directives: ["connectSrc", "imgSrc"] }),
					],
				},
			]),
			only,
		);
		expect(
			read?.purposes[0].sources.map((source) => source.directives),
		).toEqual([["connectSrc"], ["connectSrc"]]);
		expect(
			readWidgetNetwork(classified(), {
				csp: {
					connectSrc: ["https://api.cesium.com", "https://missing.example.com"],
				},
			}),
		).toBeUndefined();
		expect(
			readWidgetNetwork({ ...classified(), stale: "no" }, only),
		).toBeUndefined();
	});
});

describe("splitWidgetPolicyDescriptor", () => {
	test("network runtime sources form the runtime part with their levels", () => {
		const parsed = parseWidgetPolicyDescriptor(runtimeDescriptor());
		const split = splitWidgetPolicyDescriptor(parsed);
		expect(split.declared.policy).toEqual({
			workers: true,
			csp: {
				connectSrc: [WILDCARD, "https://api.cesium.com"],
				imgSrc: [WILDCARD],
			},
		});
		expect(split.declared.levels).toEqual({
			"https://api.cesium.com": "external",
			[WILDCARD]: "broad",
		});
		expect(split.runtime).toEqual([
			{
				directive: "connectSrc",
				source: RUNTIME_HOST,
				level: "external",
				slot: "tileUrl",
			},
			{
				directive: "imgSrc",
				source: RUNTIME_HOST,
				level: "external",
				slot: "tileUrl",
			},
		]);
	});

	test("without network the runtime part is the difference to the declared policy at the worst level", () => {
		const split = splitWidgetPolicyDescriptor(
			{ policy: effective },
			{
				workers: true,
				csp: {
					connectSrc: ["https://api.cesium.com", WILDCARD],
					imgSrc: [WILDCARD],
				},
			},
		);
		expect(
			split.runtime.map(({ directive, source, level }) => [
				directive,
				source,
				level,
			]),
		).toEqual([
			["connectSrc", RUNTIME_HOST, "broad"],
			["imgSrc", RUNTIME_HOST, "broad"],
		]);
		expect(Object.values(split.declared.levels)).toEqual(["broad", "broad"]);
		expect(splitWidgetPolicyDescriptor({ policy: effective }).runtime).toEqual(
			[],
		);
	});
});

describe("runtime requests", () => {
	test("input paths follow the contract grammar", () => {
		expect(parseWidgetInputPath("config.layers[].sources.*.url")).toEqual({
			root: "config",
			steps: [
				{ kind: "key", key: "layers" },
				{ kind: "items" },
				{ kind: "key", key: "sources" },
				{ kind: "values" },
				{ kind: "key", key: "url" },
			],
		});
		for (const path of [
			"",
			"1a",
			"a.",
			"a[",
			"a.*x",
			"a..b",
			"a.b.c.d.e.f.g",
		]) {
			expect(parseWidgetInputPath(path)).toBeNull();
		}
	});

	test("requests are canonical and keyed by content", () => {
		const request = canonicalWidgetRuntimeRequest([
			{
				slot: "b",
				sources: ["https://y.example.com", "https://x.example.com"],
			},
			{ slot: "a", sources: ["https://x.example.com"] },
			{ slot: "b", sources: ["https://x.example.com"] },
			{ slot: "c", sources: [] },
		]);
		expect(request).toEqual([
			{ slot: "a", sources: ["https://x.example.com"] },
			{
				slot: "b",
				sources: ["https://x.example.com", "https://y.example.com"],
			},
		]);
		expect(widgetRuntimeRequestSources(request)).toEqual([
			"https://x.example.com",
			"https://y.example.com",
		]);
		expect(widgetRuntimeRequestKey([...request].reverse())).toBe(
			widgetRuntimeRequestKey(request),
		);
	});
});
