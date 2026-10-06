"use client";

import type * as GeoJSON from "geojson";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useId, useMemo } from "react";
import type { Geometry } from "../../lib/geometry";
import { normalizeGeometryValue } from "../../lib/geometry";
import { IValueType } from "../../lib/schema/flow/pin";
import { Map as GeometryMap, MapControls, useMap } from "./map";

function GeometryLayers({ features }: { features: GeoJSON.FeatureCollection }) {
	const { map, isLoaded } = useMap();
	const id = useId();
	useEffect(() => {
		if (!map || !isLoaded) return;
		map.addSource(id, { type: "geojson", data: features });
		map.addLayer({
			id: `${id}-fill`,
			type: "fill",
			source: id,
			filter: ["==", "$type", "Polygon"],
			paint: { "fill-color": "#f97316", "fill-opacity": 0.25 },
		});
		map.addLayer({
			id: `${id}-line`,
			type: "line",
			source: id,
			filter: ["!=", "$type", "Point"],
			paint: { "line-color": "#f97316", "line-width": 2 },
		});
		map.addLayer({
			id: `${id}-point`,
			type: "circle",
			source: id,
			filter: ["==", "$type", "Point"],
			paint: {
				"circle-color": "#f97316",
				"circle-radius": 5,
				"circle-stroke-width": 1,
				"circle-stroke-color": "#fff",
			},
		});
		const positions: number[][] = [];
		const collect = (coords: unknown) => {
			if (!Array.isArray(coords)) return;
			if (typeof coords[0] === "number") {
				positions.push(coords);
				return;
			}
			for (const child of coords) collect(child);
		};
		for (const feature of features.features)
			if ("coordinates" in feature.geometry)
				collect(feature.geometry.coordinates);
		if (positions.length) {
			let west = 180;
			let east = -180;
			let south = 90;
			let north = -90;
			for (const [x, y] of positions) {
				west = Math.min(west, x);
				east = Math.max(east, x);
				south = Math.min(south, y);
				north = Math.max(north, y);
			}
			map.fitBounds(
				[
					[west, south],
					[east, north],
				],
				{ padding: 28, maxZoom: 13, duration: 0 },
			);
		}
		return () => {
			// The parent Map may already have removed its style during unmount.
			if (!map.getStyle()) return;
			for (const suffix of ["point", "line", "fill"])
				if (map.getLayer(`${id}-${suffix}`)) map.removeLayer(`${id}-${suffix}`);
			if (map.getSource(id)) map.removeSource(id);
		};
	}, [map, isLoaded, id, features]);
	return null;
}

export default function GeometryPreview({
	value,
	valueType = IValueType.Normal,
}: Readonly<{ value: unknown; valueType?: IValueType }>) {
	const { t } = useTranslation("flow");
	const features = useMemo((): GeoJSON.FeatureCollection | null => {
		try {
			const normalized = normalizeGeometryValue(value, { valueType });
			const geometries =
				valueType === IValueType.Normal
					? [normalized]
					: Array.isArray(normalized)
						? normalized
						: Object.values(normalized as Record<string, unknown>);
			const flattened: GeoJSON.Feature[] = [];
			const append = (geometry: Geometry) => {
				if (geometry.type === "GeometryCollection") {
					geometry.geometries.forEach(append);
					return;
				}
				flattened.push({ type: "Feature", geometry, properties: {} });
			};
			(geometries as Geometry[]).forEach(append);
			return { type: "FeatureCollection", features: flattened };
		} catch {
			return null;
		}
	}, [value, valueType]);
	if (!features || features.features.length === 0) return null;
	return (
		<div
			className="h-52 overflow-hidden rounded-md border"
			aria-label={t("geometryPreview", "Geometry map preview")}
		>
			<GeometryMap className="h-full w-full" center={[0, 0]} zoom={1}>
				<GeometryLayers features={features} />
				<MapControls />
			</GeometryMap>
		</div>
	);
}
