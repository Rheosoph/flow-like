import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { GeoJSONSource } from "maplibre-gl";
import puppeteer from "puppeteer";
import type { MapTest } from "./main";
declare global {
	interface Window {
		mapTest: MapTest;
		__mapXss?: number;
	}
}
import { prepareMapLibreAssets } from "../../packages/ui/scripts/prepare-maplibre.mjs";

const dir = await mkdtemp(join(tmpdir(), "flow-like-maplibre-"));
const workerUrl = prepareMapLibreAssets(dir, "/ui");
const build = await Bun.build({
	entrypoints: [join(import.meta.dir, "main.tsx")],
	outdir: dir,
	target: "browser",
	format: "esm",
	define: {
		"process.env.NODE_ENV": JSON.stringify("production"),
		"process.env.NEXT_PUBLIC_MAPLIBRE_WORKER_URL": JSON.stringify(workerUrl),
	},
});
assert.ok(build.success, build.logs.join("\n"));
const requests = new Set<string>();
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	async fetch(request) {
		const pathname = new URL(request.url).pathname;
		requests.add(pathname);
		if (pathname.includes("/locales/"))
			return new Response("export default {}", {
				headers: { "Content-Type": "text/javascript" },
			});
		if (pathname.endsWith(".pbf")) return new Response(new Uint8Array());
		if (pathname === "/ui/nested/map")
			return new Response(
				`<!doctype html><link rel="icon" href="data:,"><link rel="stylesheet" href="/ui/main.css"><style>.maplibregl-map { height: 300px; width: 640px; } body { margin: 0; }</style><div id="app"></div><script type="module" src="/ui/main.js"></script>`,
				{ headers: { "Content-Type": "text/html" } },
			);
		const file = Bun.file(join(dir, pathname.replace(/^\/ui\//, "")));
		if (!(await file.exists())) {
			console.error("Missing asset:", pathname);
			return new Response("Not found", { status: 404 });
		}
		return new Response(file, {
			headers: pathname.endsWith(".mjs")
				? { "Content-Type": "text/javascript" }
				: {},
		});
	},
});
const browser = await puppeteer.launch({
	headless: true,
	...(process.env.CHROME_PATH
		? { executablePath: process.env.CHROME_PATH }
		: {}),
});
try {
	const page = await browser.newPage();
	const errors: string[] = [];
	page.on("pageerror", (error) => {
		errors.push(String(error));
		console.error(String(error));
	});
	page.on("console", (message) => {
		if (message.type() === "error") {
			errors.push(message.text());
			console.error(message.text());
		}
	});
	await page.setRequestInterception(true);
	page.on("request", (request) => {
		if (request.url().startsWith(server.url.origin)) void request.continue();
		else if (request.url().endsWith("style.json"))
			void request.respond({
				status: 200,
				contentType: "application/json",
				headers: { "Access-Control-Allow-Origin": "*" },
				body: JSON.stringify({ version: 8, sources: {}, layers: [] }),
			});
		else void request.abort();
	});
	await page.goto(`${server.url.origin}/ui/nested/map`);
	await page.waitForFunction(() => {
		if (!window.mapTest.ready) return false;
		const { map, maps } = window.mapTest;
		return (
			maps.size === 2 &&
			[...maps].every((m) => m.loaded()) &&
			map.getLayer("route-layer-test") &&
			map.queryRenderedFeatures({ layers: ["route-layer-test"] }).length > 0
		);
	});
	assert.ok(requests.has(workerUrl), "versioned worker loaded under /ui");
	assert.ok(
		[...requests].some((url) => url.endsWith("maplibre-gl-shared.mjs")),
		"worker shared module loaded",
	);
	await page.click("#marker");
	await page.waitForSelector(".maplibregl-popup-content");
	assert.ok(
		(
			await page.$eval(".maplibregl-popup-content", (el) => el.textContent)
		)?.includes("Safe popup"),
	);
	await page.click("#update");
	await page.waitForFunction(async () => {
		const source = window.mapTest.map.getSource(
			"route-source-test",
		) as GeoJSONSource;
		const data = await source.getData();
		return (
			data.type === "Feature" &&
			data.geometry.type === "LineString" &&
			data.geometry.coordinates[0][0] === -2
		);
	});
	await page.evaluate(async () => {
		const { map } = window.mapTest;
		const clusterLayer = map
			.getStyle()
			.layers.find((layer) => layer.id.startsWith("clusters-"));
		if (
			!clusterLayer ||
			!("source" in clusterLayer) ||
			typeof clusterLayer.source !== "string"
		)
			throw new Error("Missing cluster layer");
		const feature = map.queryRenderedFeatures({ layers: [clusterLayer.id] })[0];
		if (feature.properties.point_count !== 3)
			throw new Error("Clustering did not return all points");
		const source = map.getSource(clusterLayer.source) as GeoJSONSource;
		const zoom = await source.getClusterExpansionZoom(
			feature.properties.cluster_id,
		);
		map.jumpTo({ center: [0, 0], zoom });
	});
	await page.waitForFunction(() => window.mapTest.moveCount > 0);
	await page.click("#theme");
	await page.waitForFunction(() => {
		const { map } = window.mapTest;
		return (
			map.loaded() &&
			map.getPaintProperty("background", "background-color") === "#182030" &&
			map.getLayer("route-layer-test")
		);
	});
	const geometryCanvas = (await page.$$("canvas.maplibregl-canvas"))[1];
	const bounds = await geometryCanvas.boundingBox();
	assert.ok(bounds);
	await page.mouse.click(
		bounds.x + bounds.width * 0.4,
		bounds.y + bounds.height * 0.4,
	);
	await page.waitForFunction(() => window.mapTest.addedPosition);
	// Exercise the real attribution control, including both adjacent-attribute bypasses.
	await page.evaluate(() => {
		const { map, maplibre } = window.mapTest;
		map.addControl(
			new maplibre.AttributionControl({
				compact: false,
				customAttribution: [
					'<details open onload="1" ontoggle="window.__mapXss=1">credit</details>',
					'<a href="javascript:window.__mapXss=2" onclick="window.__mapXss=3">credit</a>',
					'<a href="https://example.com/credit" target="_blank">Valid credit</a>',
				],
			}),
		);
	});
	const attribution = await page.$eval(
		".maplibregl-ctrl-attrib-inner",
		(el) => el.innerHTML,
	);
	assert.doesNotMatch(attribution, /onload|ontoggle|onclick|javascript:/i);
	assert.match(attribution, /https:\/\/example.com\/credit/);
	assert.equal(await page.evaluate(() => window.__mapXss), undefined);
	assert.deepEqual(await page.evaluate(() => window.mapTest.errors), []);
	await page.evaluate(() => window.mapTest.unmount());
	assert.equal(await page.evaluate(() => window.mapTest.maps.size), 0);
	assert.deepEqual(errors, []);
	console.log(
		"MapLibre browser checks passed: workers/base path, geometry, routes, clusters, popups, viewport, theme, attribution XSS, cleanup.",
	);
} catch (error) {
	const page = (await browser.pages()).at(-1);
	if (!page) throw error;
	console.error(
		await page.evaluate(() => {
			const t = window.mapTest;
			return {
				errors: t?.errors,
				mapCount: t?.maps.size,
				loaded: t?.map?.loaded(),
				layers: t?.map?.getStyle().layers.map((l) => l.id),
				background: t?.map?.getPaintProperty("background", "background-color"),
				added: t?.addedPosition,
			};
		}),
	);
	throw error;
} finally {
	await browser.close();
	server.stop(true);
	await rm(dir, { recursive: true, force: true });
}
