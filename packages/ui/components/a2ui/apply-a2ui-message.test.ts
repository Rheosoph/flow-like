/**
 * Tests for the shared a2ui reducer, focused on the setGeoMapViewport
 * normalization (backend emits shapes the GeoMap component cannot resolve
 * without it).
 */
import { describe, expect, test } from "bun:test";
import {
	applyElementUpdate,
	normalizeGeoMapViewport,
} from "./apply-a2ui-message";
import type { SurfaceComponent } from "./types";

const geoMapComponent = (): SurfaceComponent =>
	({
		id: "map-1",
		component: { type: "geoMap" },
	}) as unknown as SurfaceComponent;

describe("normalizeGeoMapViewport", () => {
	test("wraps the node's legacy flat shape into a nested-center literalJson", () => {
		const result = normalizeGeoMapViewport({
			latitude: 52.52,
			longitude: 13.405,
			zoom: 10,
		}) as { literalJson: string };

		expect(result.literalJson).toBeString();
		expect(JSON.parse(result.literalJson)).toEqual({
			center: { latitude: 52.52, longitude: 13.405 },
			zoom: 10,
		});
	});

	test("passes BoundValues through untouched", () => {
		const bound = { literalJson: '{"center":{"latitude":1,"longitude":2}}' };
		expect(normalizeGeoMapViewport(bound)).toBe(bound);

		const path = { path: "/inputs/viewport" };
		expect(normalizeGeoMapViewport(path)).toBe(path);
	});

	test("wraps an already-nested raw object", () => {
		const result = normalizeGeoMapViewport({
			center: { latitude: 1, longitude: 2 },
			bearing: 45,
		}) as { literalJson: string };

		expect(JSON.parse(result.literalJson)).toEqual({
			center: { latitude: 1, longitude: 2 },
			bearing: 45,
		});
	});

	test("returns undefined for unusable payloads", () => {
		expect(normalizeGeoMapViewport(undefined)).toBeUndefined();
		expect(normalizeGeoMapViewport({ zoom: 3 })).toBeUndefined();
	});
});

describe("applyElementUpdate setGeoMapViewport", () => {
	test("stores a resolvable BoundValue on the component", () => {
		const updated = applyElementUpdate(geoMapComponent(), {
			type: "setGeoMapViewport",
			viewport: { latitude: 48.13, longitude: 11.58, zoom: 12 },
		});

		const viewport = (updated.component as unknown as Record<string, unknown>)
			.viewport as { literalJson: string };
		expect(JSON.parse(viewport.literalJson).center).toEqual({
			latitude: 48.13,
			longitude: 11.58,
		});
	});
});
