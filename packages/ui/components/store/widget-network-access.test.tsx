import { describe, expect, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import { renderToStaticMarkup } from "react-dom/server";
import type { PackageWidgetEntry } from "../../lib/schema/wasm";
import {
	WidgetCapabilitiesCard,
	WidgetNetworkAccessCard,
	WidgetNetworkSummary,
} from "./widget-network-access";

const CESIUM_REASON =
	"Loads globe terrain, imagery and 3D tiles from Cesium ion";
const S3_REASON = "Loads map layers from Amazon S3 buckets in Frankfurt";
const TILES_REASON = "Loads map tiles from tile servers given to it at runtime";

const MAP_CONTRACT = {
	contractVersion: 2,
	id: "live-map",
	capabilities: { workers: true, downloads: true },
	csp: [
		{
			reason: CESIUM_REASON,
			connectSrc: ["https://api.cesium.com", "https://assets.ion.cesium.com"],
			imgSrc: ["https://assets.ion.cesium.com"],
		},
		{
			reason: S3_REASON,
			connectSrc: ["https://*.s3.eu-central-1.amazonaws.com"],
			imgSrc: ["https://*.s3.eu-central-1.amazonaws.com"],
		},
		{
			reason: TILES_REASON,
			inputs: [
				{
					path: "tileUrl",
					directives: ["connectSrc", "imgSrc"],
					template: { subdomainsInput: "tileSubdomains" },
				},
			],
		},
	],
} as WidgetContract;

function source(
	value: string,
	directives: string[],
	kind: string,
	level: string,
	host: string,
	emphasis: string,
	provider?: string,
) {
	return {
		source: value,
		directives,
		origin: "declared",
		kind,
		level,
		host,
		emphasis,
		...(provider ? { provider } : {}),
	};
}

const MAP_NETWORK = {
	level: "broad",
	catalogVersion: 3,
	pslVersion: "2026-09-15_10-18-26_UTC",
	stale: false,
	purposes: [
		{
			reason: CESIUM_REASON,
			level: "shared",
			sources: [
				source(
					"https://api.cesium.com",
					["connectSrc"],
					"service",
					"external",
					"api.cesium.com",
					"cesium.com",
					"Cesium ion",
				),
				source(
					"https://assets.ion.cesium.com",
					["connectSrc", "imgSrc"],
					"service",
					"shared",
					"assets.ion.cesium.com",
					"cesium.com",
					"Cesium ion",
				),
			],
		},
		{
			reason: S3_REASON,
			level: "broad",
			sources: [
				source(
					"https://*.s3.eu-central-1.amazonaws.com",
					["connectSrc", "imgSrc"],
					"shared-wildcard",
					"broad",
					"*.s3.eu-central-1.amazonaws.com",
					"s3.eu-central-1.amazonaws.com",
					"Amazon S3",
				),
			],
		},
		{
			reason: TILES_REASON,
			level: "external",
			inputs: ["tileUrl"],
			sources: [],
		},
	],
};

const GIBS_CONTRACT = {
	contractVersion: 2,
	id: "imagery",
	csp: [
		{
			reason: "Loads satellite imagery from NASA",
			imgSrc: ["https://gibs.earthdata.nasa.gov"],
		},
	],
} as WidgetContract;

const GIBS_NETWORK = {
	level: "known",
	catalogVersion: 3,
	pslVersion: "2026-09-15_10-18-26_UTC",
	stale: false,
	purposes: [
		{
			reason: "Loads satellite imagery from NASA",
			level: "known",
			sources: [
				source(
					"https://gibs.earthdata.nasa.gov",
					["imgSrc"],
					"service",
					"known",
					"gibs.earthdata.nasa.gov",
					"nasa.gov",
					"NASA GIBS",
				),
			],
		},
	],
};

const CHART_CONTRACT = {
	contractVersion: 1,
	id: "chart",
	capabilities: {},
} as WidgetContract;

function widget(
	contract: WidgetContract,
	name: string,
	network?: unknown,
): PackageWidgetEntry {
	return {
		id: contract.id,
		name,
		description: "",
		contract,
		...(network ? { network } : {}),
	};
}

function tag(markup: string, attribute: string): string {
	return markup.match(new RegExp(`<[a-z]+[^>]*${attribute}[^>]*>`))?.[0] ?? "";
}

describe("WidgetNetworkSummary", () => {
	test("renders nothing for a widget without network access", () => {
		expect(
			renderToStaticMarkup(
				<WidgetNetworkSummary
					widget={widget(CHART_CONTRACT, "Chart")}
					showPreviewNote
				/>,
			),
		).toBe("");
		expect(
			renderToStaticMarkup(
				<WidgetNetworkSummary
					widget={widget(
						{ ...CHART_CONTRACT, csp: [{ reason: "Nothing", imgSrc: [] }] },
						"Chart",
					)}
				/>,
			),
		).toBe("");
	});

	test("counts addresses with a wildcard as one, adds the level, runtime badge and preview note", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkSummary
				widget={widget(MAP_CONTRACT, "Live map", MAP_NETWORK)}
				showPreviewNote
			/>,
		);
		expect(markup).toContain("Network: 3 addresses");
		expect(tag(markup, "data-widget-level-badge")).toContain(
			'data-widget-level-badge="broad"',
		);
		expect(tag(markup, "data-widget-level-badge")).toContain("bg-destructive");
		expect(markup).toContain("Anyone can receive");
		expect(markup).toContain("data-widget-network-runtime");
		expect(markup).toContain("+ addresses provided at runtime");
		expect(markup).toContain(
			'data-widget-source-chip="*.s3.eu-central-1.amazonaws.com"',
		);
		expect(markup.match(/data-widget-source-chip=/g)).toHaveLength(3);
		expect(markup).toContain("any address under");
		expect(markup).toContain("Preview runs without network access");
	});

	test("a single wildcard reads as one address, never one site", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkSummary
				widget={widget(
					{
						...CHART_CONTRACT,
						csp: [
							{
								reason: "Loads layers",
								imgSrc: ["https://*.tiles.example.org"],
							},
						],
					},
					"Layers",
				)}
			/>,
		);
		expect(markup).toContain("Network: 1 address");
		expect(markup).not.toContain(" site");
		expect(markup).not.toContain("data-widget-network-runtime");
		expect(markup).not.toContain("Preview runs without network access");
	});

	test("calm levels never use destructive styling", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkSummary
				widget={widget(GIBS_CONTRACT, "Imagery", GIBS_NETWORK)}
			/>,
		);
		expect(markup).toContain("Network: 1 address");
		expect(markup).toContain("Identified service");
		expect(tag(markup, "data-widget-level-badge")).toContain(
			'data-widget-level-badge="known"',
		);
		expect(markup).not.toContain("bg-destructive");
		expect(markup).not.toContain("border-destructive/60");
	});

	test("without classification it shows no level word", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkSummary widget={widget(MAP_CONTRACT, "Live map")} />,
		);
		expect(markup).toContain("Network: 3 addresses");
		expect(markup).not.toContain("data-widget-level-badge");
		expect(markup).not.toContain("Anyone can receive");
		expect(tag(markup, "data-widget-network-badge")).toContain("text-primary");
	});

	test("an inputs-only widget shows only the runtime badge", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkSummary
				widget={widget(
					{
						...CHART_CONTRACT,
						csp: [MAP_CONTRACT.csp?.[2]],
					} as WidgetContract,
					"Tiles",
				)}
			/>,
		);
		expect(markup).toContain("+ addresses provided at runtime");
		expect(markup).not.toContain("data-widget-network-badge");
	});
});

describe("WidgetNetworkAccessCard", () => {
	test("renders purpose cards with levels, sources, runtime slots and publisher reasons", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkAccessCard
				widgets={[
					widget(MAP_CONTRACT, "Live map", MAP_NETWORK),
					widget(CHART_CONTRACT, "Chart"),
				]}
			/>,
		);
		expect(markup).toContain("Widget network access");
		expect(markup.match(/data-widget-purpose="true"/g)).toHaveLength(3);
		const levels = [
			...markup.matchAll(/data-widget-purpose-level="([a-z]+)"/g),
		].map((match) => match[1]);
		expect(levels).toEqual(["broad", "shared", "external"]);
		expect(markup).toContain("Anyone can receive");
		expect(markup).toContain("Hosts user content");
		expect(markup).toContain("External site");
		expect(markup).toContain(
			"Matches every customer under s3.eu-central-1.amazonaws.com. Anyone who signs up there could receive what the widget sends.",
		);
		expect(markup).toContain(
			"Data goes to Cesium ion. Anyone with a Cesium ion account could receive what the widget sends there.",
		);
		expect(markup).toContain("Amazon S3");
		expect(markup).toContain("Send and receive data · Load images");
		expect(markup).toContain("Addresses provided while the app runs");
		expect(markup).toContain('data-widget-runtime-slot="tileUrl"');
		expect(markup).toContain("Subdomains from input");
		expect(markup).toContain(">tileSubdomains</code>");
		expect(markup).toContain(
			"Each address is checked and shown to the viewer for approval when it appears.",
		);
		expect(markup).toContain("Publisher:");
		expect(markup).toContain(`<bdi>${S3_REASON}</bdi>`);
		expect(markup.lastIndexOf("data-widget-host-copy")).toBeLessThan(
			markup.lastIndexOf("data-widget-publisher-note"),
		);
		expect(markup).toContain("No network access");
	});

	test("keeps declaration order and omits levels without classification", () => {
		const markup = renderToStaticMarkup(
			<WidgetNetworkAccessCard widgets={[widget(MAP_CONTRACT, "Live map")]} />,
		);
		expect(markup).not.toContain("data-widget-purpose-level");
		const reasons = [...markup.matchAll(/<bdi>([^<]+)<\/bdi>/g)].map(
			(match) => match[1],
		);
		expect(reasons).toEqual([CESIUM_REASON, S3_REASON, TILES_REASON]);
		expect(markup).toContain('data-widget-source-chip="api.cesium.com"');
		expect(markup).not.toContain("Data goes to");
	});

	test("renders nothing for a package without widgets", () => {
		expect(renderToStaticMarkup(<WidgetNetworkAccessCard widgets={[]} />)).toBe(
			"",
		);
	});
});

describe("WidgetCapabilitiesCard", () => {
	test("shows review flags, source and input tables, reasons and data version", () => {
		const markup = renderToStaticMarkup(
			<WidgetCapabilitiesCard
				widgets={[
					widget(MAP_CONTRACT, "Live map", MAP_NETWORK),
					widget(CHART_CONTRACT, "Chart"),
				]}
			/>,
		);
		expect(markup).toContain("Widget capabilities");
		expect(markup).toContain('data-widget-capability="workers"');
		expect(markup).toContain('data-widget-capability="downloads"');
		expect(markup).toContain("Check before approving");
		const flags = [
			...markup.matchAll(/data-widget-review-flag="([a-z-]+)"/g),
		].map((match) => match[1]);
		expect(flags).toEqual([
			"broad",
			"shared",
			"runtime-connect",
			"platform-storage",
		]);
		expect(markup).toContain(
			"https://*.s3.eu-central-1.amazonaws.com: anyone who signs up with Amazon S3 could receive what the widget sends.",
		);
		expect(markup).toContain(
			"https://assets.ion.cesium.com: anyone can publish content the widget loads.",
		);
		expect(markup).toContain(
			"Input tileUrl can add sites the widget sends data to.",
		);
		expect(markup).toContain("data-widget-review-sources");
		expect(markup).toContain("connect-src, img-src");
		expect(markup).toContain("Every customer of a shared platform");
		expect(markup).toContain('data-widget-level-badge="broad"');
		expect(markup).toContain("Cesium ion");
		expect(markup).toContain("data-widget-review-inputs");
		expect(markup).toContain("subdomains from tileSubdomains");
		expect(markup).toContain(CESIUM_REASON);
		expect(markup).toContain(
			"Classified with catalog 3 and public suffix list 2026-09-15_10-18-26_UTC",
		);
		expect(markup).toContain("Declares no capabilities or network addresses");
	});

	test("without classification only input flags remain and levels are blank", () => {
		const markup = renderToStaticMarkup(
			<WidgetCapabilitiesCard widgets={[widget(MAP_CONTRACT, "Live map")]} />,
		);
		const flags = [
			...markup.matchAll(/data-widget-review-flag="([a-z-]+)"/g),
		].map((match) => match[1]);
		expect(flags).toEqual(["runtime-connect", "platform-storage"]);
		expect(markup).not.toContain("data-widget-level-badge");
		expect(markup).toContain("Unknown");
		expect(markup).not.toContain("Classified with catalog");
	});
});
