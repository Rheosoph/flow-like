"use client";

import type * as GeoJSON from "geojson";

import { useTranslation } from "@flow-like/locales";
import type { GeoJSONSource, MapMouseEvent } from "maplibre-gl";
import { type MutableRefObject, useEffect, useId, useMemo, useRef } from "react";
import {
	type GeometryDraft,
	type GeometryPosition,
	type GeometryRingRef,
	clampPosition,
	draftCapabilities,
	draftPositionCount,
} from "../../lib/geometry-draft";
import { cn } from "../../lib/utils";
import {
	Map as GeometryMap,
	MapControls,
	MapMarker,
	MarkerContent,
	useMap,
} from "./map";

/** Above this, vertices render as a circle layer and are edited from the list. */
const MAX_VERTEX_MARKERS = 500;
const COLOR = "#f97316";

export interface GeometryEditorMapProps {
	draft: GeometryDraft;
	active: GeometryRingRef;
	disabled?: boolean;
	onAddPosition: (position: GeometryPosition) => void;
	onMovePosition: (
		ref: GeometryRingRef,
		index: number,
		position: GeometryPosition,
		commit: boolean,
	) => void;
	onSelectRing: (ref: GeometryRingRef) => void;
	/** Latest map center, so "Add vertex" can place into the visible area. */
	centerRef?: MutableRefObject<GeometryPosition | null>;
}

function draftFeatures(
	draft: GeometryDraft,
	active: GeometryRingRef,
): GeoJSON.FeatureCollection {
	const { polygon } = draftCapabilities(draft.kind);
	const features: GeoJSON.Feature[] = [];
	draft.shapes.forEach((shape, s) => {
		const activeShape = s === active.shape;
		if (polygon && shape[0].length >= 3) {
			features.push({
				type: "Feature",
				properties: { role: "fill", active: activeShape },
				geometry: {
					type: "Polygon",
					coordinates: shape
						.filter((ring) => ring.length >= 3)
						.map((ring) => [...ring, ring[0]]),
				},
			});
		}
		shape.forEach((ring, r) => {
			const isActive = activeShape && r === active.ring;
			if (ring.length >= 2) {
				features.push({
					type: "Feature",
					properties: { role: "line", active: isActive },
					geometry: {
						type: "LineString",
						coordinates:
							polygon && ring.length >= 3 ? [...ring, ring[0]] : ring,
					},
				});
			}
			for (const position of ring) {
				features.push({
					type: "Feature",
					properties: { role: "vertex", active: isActive },
					geometry: { type: "Point", coordinates: position },
				});
			}
		});
	});
	return { type: "FeatureCollection", features };
}

function draftBounds(
	draft: GeometryDraft,
): [[number, number], [number, number]] | null {
	let west = 180;
	let east = -180;
	let south = 90;
	let north = -90;
	let any = false;
	for (const shape of draft.shapes)
		for (const ring of shape)
			for (const [x, y] of ring) {
				any = true;
				west = Math.min(west, x);
				east = Math.max(east, x);
				south = Math.min(south, y);
				north = Math.max(north, y);
			}
	return any
		? [
				[west, south],
				[east, north],
			]
		: null;
}

function DraftLayers({
	draft,
	active,
	showVertices,
}: {
	draft: GeometryDraft;
	active: GeometryRingRef;
	showVertices: boolean;
}) {
	const { map, isLoaded } = useMap();
	const id = useId();
	const features = useMemo(() => draftFeatures(draft, active), [draft, active]);
	const featuresRef = useRef(features);
	featuresRef.current = features;
	const fitted = useRef(false);

	useEffect(() => {
		if (!map || !isLoaded) return;
		map.addSource(id, { type: "geojson", data: featuresRef.current });
		map.addLayer({
			id: `${id}-fill`,
			type: "fill",
			source: id,
			filter: ["==", ["get", "role"], "fill"],
			paint: {
				"fill-color": COLOR,
				"fill-opacity": ["case", ["get", "active"], 0.28, 0.14],
			},
		});
		map.addLayer({
			id: `${id}-line`,
			type: "line",
			source: id,
			filter: ["==", ["get", "role"], "line"],
			paint: {
				"line-color": COLOR,
				"line-width": ["case", ["get", "active"], 3, 2],
				"line-opacity": ["case", ["get", "active"], 1, 0.6],
			},
		});
		map.addLayer({
			id: `${id}-vertex`,
			type: "circle",
			source: id,
			filter: ["==", ["get", "role"], "vertex"],
			paint: {
				"circle-color": COLOR,
				"circle-radius": 3,
				"circle-stroke-width": 1,
				"circle-stroke-color": "#fff",
			},
		});
		return () => {
			// The parent Map may already have removed its style during unmount.
			if (!map.getStyle()) return;
			for (const suffix of ["vertex", "line", "fill"])
				if (map.getLayer(`${id}-${suffix}`)) map.removeLayer(`${id}-${suffix}`);
			if (map.getSource(id)) map.removeSource(id);
		};
	}, [map, isLoaded, id]);

	useEffect(() => {
		if (!map || !isLoaded) return;
		const source = map.getSource(id) as GeoJSONSource | undefined;
		source?.setData(features);
		if (map.getLayer(`${id}-vertex`))
			map.setLayoutProperty(
				`${id}-vertex`,
				"visibility",
				showVertices ? "visible" : "none",
			);
	}, [map, isLoaded, id, features, showVertices]);

	useEffect(() => {
		if (!map || !isLoaded || fitted.current) return;
		const bounds = draftBounds(draft);
		if (!bounds) return;
		fitted.current = true;
		map.fitBounds(bounds, { padding: 40, maxZoom: 13, duration: 0 });
	}, [map, isLoaded, draft]);

	return null;
}

function DraftInteractions({
	disabled,
	onAddPosition,
	centerRef,
}: Pick<GeometryEditorMapProps, "disabled" | "onAddPosition" | "centerRef">) {
	const { map } = useMap();
	const addRef = useRef(onAddPosition);
	addRef.current = onAddPosition;

	useEffect(() => {
		if (!map || !centerRef) return;
		const track = () => {
			const center = map.getCenter().wrap();
			centerRef.current = clampPosition([center.lng, center.lat]);
		};
		track();
		map.on("moveend", track);
		return () => {
			map.off("moveend", track);
		};
	}, [map, centerRef]);

	useEffect(() => {
		if (!map || disabled) return;
		const handleClick = (event: MapMouseEvent) => {
			const { lng, lat } = event.lngLat.wrap();
			addRef.current(clampPosition([lng, lat]));
		};
		map.on("click", handleClick);
		const canvas = map.getCanvas();
		const previousCursor = canvas.style.cursor;
		canvas.style.cursor = "crosshair";
		return () => {
			map.off("click", handleClick);
			canvas.style.cursor = previousCursor;
		};
	}, [map, disabled]);

	return null;
}

function VertexMarkers({
	draft,
	active,
	disabled,
	onMovePosition,
	onSelectRing,
}: Omit<GeometryEditorMapProps, "onAddPosition" | "centerRef">) {
	const markers: React.ReactNode[] = [];
	draft.shapes.forEach((shape, s) =>
		shape.forEach((ring, r) => {
			const ref = { shape: s, ring: r };
			const isActive = s === active.shape && r === active.ring;
			ring.forEach((position, index) =>
				markers.push(
					<MapMarker
						key={`${s}:${r}:${index}`}
						longitude={position[0]}
						latitude={position[1]}
						draggable={!disabled}
						onClick={() => onSelectRing(ref)}
						onDrag={({ lng, lat }) =>
							onMovePosition(ref, index, [lng, lat], false)
						}
						onDragEnd={({ lng, lat }) =>
							onMovePosition(ref, index, [lng, lat], true)
						}
					>
						<MarkerContent
							className={disabled ? "cursor-default" : "cursor-grab"}
						>
							<span
								className={cn(
									"block size-3 rounded-full border-2 border-white shadow",
									isActive ? "bg-orange-500" : "bg-orange-300",
								)}
							/>
						</MarkerContent>
					</MapMarker>,
				),
			);
		}),
	);
	return <>{markers}</>;
}

export default function GeometryEditorMap({
	draft,
	active,
	disabled,
	onAddPosition,
	onMovePosition,
	onSelectRing,
	centerRef,
}: Readonly<GeometryEditorMapProps>) {
	const { t } = useTranslation("flow");
	const markers = draftPositionCount(draft) <= MAX_VERTEX_MARKERS;
	return (
		<div
			className="relative h-64 overflow-hidden rounded-md border"
			aria-label={t("geometryEditorMap", "Geometry map editor")}
		>
			<GeometryMap
				className="h-full w-full"
				center={[0, 0]}
				zoom={1}
				doubleClickZoom={false}
			>
				<DraftLayers draft={draft} active={active} showVertices={!markers} />
				<DraftInteractions
					disabled={disabled}
					onAddPosition={onAddPosition}
					centerRef={centerRef}
				/>
				{markers && (
					<VertexMarkers
						draft={draft}
						active={active}
						disabled={disabled}
						onMovePosition={onMovePosition}
						onSelectRing={onSelectRing}
					/>
				)}
				<MapControls />
			</GeometryMap>
		</div>
	);
}
