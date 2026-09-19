import { describe, expect, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
	WIDGET_NET_CAPABILITY,
	capabilitySeverity,
	describePackageWidgetNetwork,
	flattenWidgetPurposes,
	readWidgetPurposes,
	usePackageCapabilities,
	widgetNetworkHosts,
	widgetReviewFlags,
} from "./package-capabilities";

function contract(csp: unknown): WidgetContract {
	return { contractVersion: 2, id: "live-map", csp } as WidgetContract;
}

function contractCsp(csp: unknown) {
	return flattenWidgetPurposes(readWidgetPurposes(contract(csp)));
}

const MAP_PURPOSES = [
	{
		reason: "Loads globe terrain, imagery and 3D tiles from Cesium ion",
		connectSrc: ["https://api.cesium.com", "https://assets.ion.cesium.com"],
		imgSrc: ["https://assets.ion.cesium.com"],
	},
	{
		reason: "Loads map layers from Amazon S3 buckets in Frankfurt",
		connectSrc: ["https://*.s3.eu-central-1.amazonaws.com"],
		imgSrc: ["https://*.s3.eu-central-1.amazonaws.com"],
	},
	{
		reason: "Loads map tiles from tile servers given to it at runtime",
		inputs: [
			{
				path: "tileUrl",
				directives: ["imgSrc", "connectSrc"],
				template: { subdomainsInput: "tileSubdomains" },
			},
		],
	},
];

const MAP_NETWORK = {
	level: "broad",
	catalogVersion: 1,
	pslVersion: "2026-09-15_10-18-26_UTC",
	stale: false,
	purposes: [
		{
			reason: MAP_PURPOSES[0].reason,
			level: "shared",
			sources: [
				{
					source: "https://api.cesium.com",
					directives: ["connectSrc"],
					origin: "declared",
					kind: "service",
					level: "external",
					host: "api.cesium.com",
					emphasis: "cesium.com",
					provider: "Cesium ion",
					aboutKey: "widgetSourceAboutMapData",
				},
				{
					source: "https://assets.ion.cesium.com",
					directives: ["connectSrc", "imgSrc"],
					origin: "declared",
					kind: "service",
					level: "shared",
					host: "assets.ion.cesium.com",
					emphasis: "cesium.com",
					provider: "Cesium ion",
				},
			],
		},
		{
			reason: MAP_PURPOSES[1].reason,
			level: "broad",
			sources: [
				{
					source: "https://*.s3.eu-central-1.amazonaws.com",
					directives: ["connectSrc", "imgSrc"],
					origin: "declared",
					kind: "shared-wildcard",
					level: "broad",
					host: "*.s3.eu-central-1.amazonaws.com",
					emphasis: "s3.eu-central-1.amazonaws.com",
					provider: "Amazon S3",
				},
			],
		},
		{
			reason: MAP_PURPOSES[2].reason,
			level: "external",
			inputs: ["tileUrl"],
			sources: [],
		},
	],
};

describe("capabilitySeverity", () => {
	test("widget network access is elevated like package network access", () => {
		expect(WIDGET_NET_CAPABILITY).toBe("widget.net");
		expect(capabilitySeverity("widget.net")).toBe("elevated");
		expect(capabilitySeverity("net.http")).toBe("elevated");
	});

	test("sandbox-internal and unknown tags stay standard", () => {
		expect(capabilitySeverity("a2ui")).toBe("standard");
		expect(capabilitySeverity("storage.node")).toBe("standard");
		expect(capabilitySeverity("widget.unknown")).toBe("standard");
	});

	test("the widget.net tag has a store label", () => {
		function Labels() {
			const capabilities = usePackageCapabilities([WIDGET_NET_CAPABILITY]);
			return createElement(
				"ul",
				null,
				capabilities.map((capability) =>
					createElement(
						"li",
						{ key: capability.key, "data-severity": capability.severity },
						capability.label,
					),
				),
			);
		}
		const markup = renderToStaticMarkup(createElement(Labels));
		expect(markup).toContain('data-severity="elevated"');
		expect(markup).toContain("Has widgets that ask to reach external sites");
		expect(markup).not.toContain(">widget.net<");
	});
});

describe("readWidgetPurposes", () => {
	test("keeps declaration order with sorted sources and inputs", () => {
		const purposes = readWidgetPurposes(
			contract([
				{
					reason: "Loads tiles",
					imgSrc: [
						"https://b.tile.openstreetmap.org",
						"https://a.tile.openstreetmap.org",
						"https://a.tile.openstreetmap.org",
					],
					inputs: [
						{ path: "overlay", directives: ["imgSrc"] },
						{
							path: "base",
							directives: ["imgSrc", "connectSrc"],
							template: { subdomains: ["b", "a"] },
						},
					],
				},
				{ reason: "Streams updates", connectSrc: ["wss://live.example.com"] },
			]),
		);
		expect(purposes).toEqual([
			{
				reason: "Loads tiles",
				csp: {
					imgSrc: [
						"https://a.tile.openstreetmap.org",
						"https://b.tile.openstreetmap.org",
					],
				},
				inputs: [
					{
						path: "base",
						directives: ["connectSrc", "imgSrc"],
						template: { subdomains: ["a", "b"] },
					},
					{ path: "overlay", directives: ["imgSrc"] },
				],
			},
			{
				reason: "Streams updates",
				csp: { connectSrc: ["wss://live.example.com"] },
				inputs: [],
			},
		]);
	});

	test("skips malformed publisher input instead of rendering it", () => {
		expect(
			readWidgetPurposes(
				contract([
					"https://x.example.com",
					{
						reason: "Loads images",
						connectSrc: "https://api.example.com",
						imgSrc: [42, "", "https://img.example.com"],
						scriptSrc: ["https://cdn.example.com"],
						inputs: [{ path: "", directives: ["imgSrc"] }, { path: "x" }],
					},
					{ reason: "Nothing usable", fontSrc: [] },
				]),
			),
		).toEqual([
			{
				reason: "Loads images",
				csp: { imgSrc: ["https://img.example.com"] },
				inputs: [],
			},
		]);
		expect(
			readWidgetPurposes(contract({ connectSrc: ["https://a.b"] })),
		).toEqual([]);
		expect(readWidgetPurposes(contract(undefined))).toEqual([]);
		expect(readWidgetPurposes(null)).toEqual([]);
	});
});

describe("flattened purposes", () => {
	test("union per directive in directive order across purposes", () => {
		expect(contractCsp(MAP_PURPOSES)).toEqual({
			connectSrc: [
				"https://*.s3.eu-central-1.amazonaws.com",
				"https://api.cesium.com",
				"https://assets.ion.cesium.com",
			],
			imgSrc: [
				"https://*.s3.eu-central-1.amazonaws.com",
				"https://assets.ion.cesium.com",
			],
		});
	});

	test("counts a host once across schemes and directives, and a wildcard once", () => {
		expect(
			widgetNetworkHosts({
				csp: contractCsp([
					{
						reason: "Streams updates",
						connectSrc: ["https://live.example.com", "wss://live.example.com"],
						imgSrc: ["https://live.example.com"],
					},
					{
						reason: "Loads layers",
						imgSrc: ["https://*.tiles.example.org"],
					},
				]),
			}),
		).toEqual(["*.tiles.example.org", "live.example.com"]);
		expect(widgetNetworkHosts({ csp: contractCsp(undefined) })).toEqual([]);
	});
});

describe("describePackageWidgetNetwork", () => {
	test("joins declared purposes with the hub classification", () => {
		const view = describePackageWidgetNetwork({
			contract: contract(MAP_PURPOSES),
			network: MAP_NETWORK,
		});
		expect(view.network?.level).toBe("broad");
		expect(view.hosts).toEqual([
			"*.s3.eu-central-1.amazonaws.com",
			"api.cesium.com",
			"assets.ion.cesium.com",
		]);
		expect(view.hasInputs).toBe(true);
		expect(view.purposes.map((purpose) => purpose.level)).toEqual([
			"shared",
			"broad",
			"external",
		]);
		expect(
			view.purposes[0].sources.map((source) => [
				source.source,
				source.directives,
				source.class?.level,
			]),
		).toEqual([
			["https://api.cesium.com", ["connectSrc"], "external"],
			["https://assets.ion.cesium.com", ["connectSrc", "imgSrc"], "shared"],
		]);
		expect(view.purposes[2].inputs).toEqual([
			{
				path: "tileUrl",
				directives: ["connectSrc", "imgSrc"],
				template: { subdomainsInput: "tileSubdomains" },
			},
		]);
	});

	test("falls back without levels when the block is absent or misstates the contract", () => {
		const absent = describePackageWidgetNetwork({
			contract: contract(MAP_PURPOSES),
		});
		expect(absent.network).toBeUndefined();
		expect(
			absent.purposes.every((purpose) => purpose.level === undefined),
		).toBe(true);
		expect(absent.purposes[1].sources[0].class).toBeUndefined();
		expect(
			describePackageWidgetNetwork({
				contract: contract([
					...MAP_PURPOSES,
					{ reason: "Unclassified", imgSrc: ["https://x.example.com"] },
				]),
				network: MAP_NETWORK,
			}).network,
		).toBeUndefined();
	});

	test("an inputs-only widget has no hosts but runtime inputs", () => {
		const view = describePackageWidgetNetwork({
			contract: contract([MAP_PURPOSES[2]]),
		});
		expect(view.hosts).toEqual([]);
		expect(view.hasInputs).toBe(true);
	});
});

describe("widgetReviewFlags", () => {
	test("flags broad and shared sources, sending inputs and every input's storage reach", () => {
		const { purposes } = describePackageWidgetNetwork({
			contract: contract(MAP_PURPOSES),
			network: MAP_NETWORK,
		});
		expect(widgetReviewFlags(purposes)).toEqual([
			{
				kind: "broad",
				source: "https://*.s3.eu-central-1.amazonaws.com",
				provider: "Amazon S3",
			},
			{ kind: "shared", source: "https://assets.ion.cesium.com" },
			{ kind: "runtime-connect", input: "tileUrl" },
			{ kind: "platform-storage", input: "tileUrl" },
		]);

		const imagesOnly = describePackageWidgetNetwork({
			contract: contract([
				{
					reason: "Shows stored map layers",
					inputs: [{ path: "files[].url", directives: ["imgSrc"] }],
				},
			]),
		});
		expect(widgetReviewFlags(imagesOnly.purposes)).toEqual([
			{ kind: "platform-storage", input: "files[].url" },
		]);
	});

	test("names the host as provider when the catalog has none, and skips calm sources", () => {
		const purposes = [
			{
				reason: "Receives hooks",
				level: "broad" as const,
				sources: [
					{
						source: "https://webhook.site",
						directives: ["connectSrc" as const],
						class: {
							source: "https://webhook.site",
							directives: ["connectSrc" as const],
							origin: "declared" as const,
							kind: "shared-host",
							level: "broad" as const,
							host: "webhook.site",
							emphasis: "webhook.site",
						},
					},
					{
						source: "https://gibs.earthdata.nasa.gov",
						directives: ["imgSrc" as const],
						class: {
							source: "https://gibs.earthdata.nasa.gov",
							directives: ["imgSrc" as const],
							origin: "declared" as const,
							kind: "service",
							level: "known" as const,
							host: "gibs.earthdata.nasa.gov",
							emphasis: "nasa.gov",
							provider: "NASA GIBS",
						},
					},
				],
				inputs: [{ path: "overlay", directives: ["imgSrc" as const] }],
			},
		];
		expect(widgetReviewFlags(purposes)).toEqual([
			{
				kind: "broad",
				source: "https://webhook.site",
				provider: "webhook.site",
			},
			{ kind: "platform-storage", input: "overlay" },
		]);
	});
});
