"use client";

import type * as GeoJSON from "geojson";
import { useEffect, useId, useMemo } from "react";
import { cn } from "../../lib/utils";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
import {
	MapControls,
	MapMarker,
	Map as MapView,
	MarkerContent,
	MarkerTooltip,
	useMap,
} from "../ui/map";
import { memoryKindStyle } from "./bot-memory-kinds";
import { type MemoryPlace, memoryPlace } from "./bot-memory-model";

interface LocatedFact {
	fact: BotFact;
	place: MemoryPlace;
}

function reducedMotion() {
	return (
		typeof window !== "undefined" &&
		window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
	);
}

function boundsOf(places: readonly MemoryPlace[]) {
	let west = 180;
	let east = -180;
	let south = 90;
	let north = -90;
	for (const { bounds } of places) {
		west = Math.min(west, bounds[0][0]);
		south = Math.min(south, bounds[0][1]);
		east = Math.max(east, bounds[1][0]);
		north = Math.max(north, bounds[1][1]);
	}
	return [
		[west, south],
		[east, north],
	] as [[number, number], [number, number]];
}

function Shapes({
	located,
	selectedId,
}: Readonly<{ located: readonly LocatedFact[]; selectedId?: string | null }>) {
	const { map, isLoaded } = useMap();
	const id = useId();
	const data = useMemo((): GeoJSON.FeatureCollection => {
		const features: GeoJSON.Feature[] = [];
		const append = (fact: BotFact, geometry: GeoJSON.Geometry) => {
			if (geometry.type === "GeometryCollection") {
				for (const member of geometry.geometries) append(fact, member);
				return;
			}
			if (geometry.type === "Point" || geometry.type === "MultiPoint") return;
			features.push({
				type: "Feature",
				geometry,
				properties: {
					color: memoryKindStyle(fact.kind).hex,
					selected: fact.id === selectedId,
				},
			});
		};
		for (const { fact, place } of located) append(fact, place.geometry);
		return { type: "FeatureCollection", features };
	}, [located, selectedId]);

	useEffect(() => {
		if (!map || !isLoaded) return;
		map.addSource(id, {
			type: "geojson",
			data: { type: "FeatureCollection", features: [] },
		});
		map.addLayer({
			id: `${id}-fill`,
			type: "fill",
			source: id,
			filter: ["==", "$type", "Polygon"],
			paint: {
				"fill-color": ["get", "color"],
				"fill-opacity": ["case", ["get", "selected"], 0.32, 0.18],
			},
		});
		map.addLayer({
			id: `${id}-line`,
			type: "line",
			source: id,
			layout: { "line-cap": "round", "line-join": "round" },
			paint: {
				"line-color": ["get", "color"],
				"line-width": ["case", ["get", "selected"], 3.5, 2],
			},
		});
		return () => {
			if (!map.getStyle()) return;
			for (const suffix of ["line", "fill"])
				if (map.getLayer(`${id}-${suffix}`)) map.removeLayer(`${id}-${suffix}`);
			if (map.getSource(id)) map.removeSource(id);
		};
	}, [map, isLoaded, id]);

	useEffect(() => {
		if (!map || !isLoaded) return;
		const source = map.getSource(id);
		if (source && "setData" in source)
			(
				source as { setData: (value: GeoJSON.FeatureCollection) => void }
			).setData(data);
	}, [map, isLoaded, id, data]);

	return null;
}

/** Room for the pins themselves, the zoom buttons on the right and the attribution along the bottom. */
const CHROME_PADDING = { top: 40, left: 40, right: 64, bottom: 72 };

/** Camera moves don't need the style, so framing works before (or without) the basemap. */
function Framing({
	located,
	selectedId,
}: Readonly<{ located: readonly LocatedFact[]; selectedId?: string | null }>) {
	const { map } = useMap();
	// Keyed by value: a parent re-render with equal places must not undo the viewer's panning.
	const frame = JSON.stringify(boundsOf(located.map(({ place }) => place)));
	const selected = located.find(({ fact }) => fact.id === selectedId)?.place;
	const focus = selected ? JSON.stringify(selected.bounds) : null;

	useEffect(() => {
		if (!map) return;
		map.fitBounds(JSON.parse(frame), {
			padding: CHROME_PADDING,
			maxZoom: 13,
			duration: 0,
		});
	}, [map, frame]);

	useEffect(() => {
		if (!map || !focus) return;
		map.fitBounds(JSON.parse(focus), {
			padding: { ...CHROME_PADDING, top: 64, left: 64 },
			maxZoom: 14,
			duration: reducedMotion() ? 0 : 700,
		});
	}, [map, focus]);

	return null;
}

/** The credit stays one tap away instead of covering a small map. */
function FoldedAttribution() {
	const { map } = useMap();
	useEffect(() => {
		if (!map) return;
		const fold = () => {
			const credit = map
				.getContainer()
				.querySelector(".maplibregl-ctrl-attrib.maplibregl-compact-show");
			credit?.classList.remove("maplibregl-compact-show");
			credit?.removeAttribute("open");
		};
		map.once("idle", fold);
		return () => {
			map.off("idle", fold);
		};
	}, [map]);
	return null;
}

function Pin({
	fact,
	selected,
}: Readonly<{ fact: BotFact; selected: boolean }>) {
	const { icon: Icon, solid } = memoryKindStyle(fact.kind);
	return (
		<span className="relative flex flex-col items-center">
			<span
				className={cn(
					"flex size-7 items-center justify-center rounded-full border-2 border-white text-white shadow-md shadow-black/25 transition-transform duration-200 motion-reduce:transition-none",
					solid,
					selected && "scale-125",
				)}
			>
				<Icon className="size-3.5" strokeWidth={2.25} aria-hidden />
			</span>
			{selected && (
				<span
					aria-hidden
					className={cn(
						"absolute top-1/2 left-1/2 -z-10 size-12 -translate-x-1/2 -translate-y-1/2 rounded-full opacity-25 motion-safe:animate-ping",
						solid,
					)}
				/>
			)}
		</span>
	);
}

/** Every remembered place on one map, pinned in its memory's color. */
export default function MemoryMap({
	facts,
	selectedId,
	onSelect,
	className,
}: Readonly<{
	facts: readonly BotFact[];
	selectedId?: string | null;
	onSelect?: (id: string) => void;
	className?: string;
}>) {
	const located = useMemo(
		() =>
			facts.flatMap((fact) => {
				const place = memoryPlace(fact.geojson);
				return place ? [{ fact, place }] : [];
			}),
		[facts],
	);
	if (located.length === 0) return null;
	return (
		<div className={cn("relative overflow-hidden", className)}>
			<MapView
				className="size-full"
				center={located[0].place.center}
				zoom={3}
				attributionControl={{ compact: true }}
			>
				<Shapes located={located} selectedId={selectedId} />
				{located.map(({ fact, place }) => (
					<MapMarker
						key={fact.id}
						longitude={place.center[0]}
						latitude={place.center[1]}
						onClick={() => onSelect?.(fact.id)}
					>
						<MarkerContent>
							<Pin fact={fact} selected={fact.id === selectedId} />
						</MarkerContent>
						{onSelect && (
							<MarkerTooltip className="max-w-56 text-xs leading-snug">
								{fact.text}
							</MarkerTooltip>
						)}
					</MapMarker>
				))}
				<Framing located={located} selectedId={selectedId} />
				<FoldedAttribution />
				<MapControls position="bottom-right" showZoom />
			</MapView>
		</div>
	);
}
