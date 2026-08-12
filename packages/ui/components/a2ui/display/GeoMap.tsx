"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	Map,
	MapControls,
	MapMarker,
	MapPopup,
	type MapRef,
	MapRoute,
	MarkerContent,
	MarkerLabel,
} from "../../ui/map";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type {
	BoundValue,
	GeoCoordinate,
	GeoMapComponent,
	GeoMapMarkerDef,
	GeoMapRouteDef,
	GeoMapViewport,
} from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function toMapCoord(c: GeoCoordinate): [number, number] {
	return [c.longitude, c.latitude];
}

function routeToLngLat(coords: GeoCoordinate[]): [number, number][] {
	return coords.map(toMapCoord);
}

const MARKER_COLORS: Record<string, string> = {
	red: "bg-red-500",
	blue: "bg-blue-500",
	green: "bg-green-500",
	yellow: "bg-yellow-500",
	orange: "bg-orange-500",
	purple: "bg-purple-500",
	pink: "bg-pink-500",
	gray: "bg-gray-500",
};

function MarkerDot({ color }: { color?: string }) {
	const colorClass =
		color && MARKER_COLORS[color] ? MARKER_COLORS[color] : "bg-blue-500";
	return (
		<div
			className={cn(
				"relative h-4 w-4 rounded-full border-2 border-white shadow-lg",
				colorClass,
			)}
		/>
	);
}

export function A2UIGeoMap({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
}: ComponentProps<GeoMapComponent>) {
	const triggerEvent = useComponentEventTrigger(componentId);
	const viewport = useResolved<GeoMapViewport>(component.viewport);
	const markers = useResolved<GeoMapMarkerDef[]>(component.markers);
	const routes = useResolved<GeoMapRouteDef[]>(component.routes);
	const showControls = useResolved<boolean>(component.showControls) ?? true;
	const showZoom = useResolved<boolean>(component.showZoom) ?? true;
	const showCompass = useResolved<boolean>(component.showCompass) ?? false;
	const showLocate = useResolved<boolean>(component.showLocate) ?? false;
	const showFullscreen =
		useResolved<boolean>(component.showFullscreen) ?? false;
	const interactive = useResolved<boolean>(component.interactive) ?? true;
	const controlPosition =
		useResolved<string>(component.controlPosition) ?? "bottom-right";

	const [activePopupId, setActivePopupId] = useState<string | null>(null);

	const mapRef = useRef<MapRef | null>(null);
	const programmaticMoveRef = useRef(false);
	const flightIdRef = useRef(0);

	// resolve() re-parses literalJson into a fresh object every render, so
	// viewport changes are detected by value, not identity.
	const viewportKey = useMemo(
		() => (viewport?.center ? JSON.stringify(viewport) : ""),
		[viewport],
	);
	const initialViewportKeyRef = useRef(viewportKey);
	const sawFirstViewportRef = useRef(false);

	// Fly (animated) to viewport updates streamed after the initial render.
	// The map itself stays uncontrolled: the controlled-mode sync would jump
	// without animation and snap back after user pans.
	useEffect(() => {
		if (!viewportKey) return;
		if (!sawFirstViewportRef.current) {
			sawFirstViewportRef.current = true;
			// The map is constructed at the initial viewport — no flight needed.
			if (viewportKey === initialViewportKeyRef.current) return;
		}

		const map = mapRef.current;
		if (!map) return;

		const next = JSON.parse(viewportKey) as GeoMapViewport;
		const flightId = ++flightIdRef.current;
		programmaticMoveRef.current = true;
		map.flyTo({
			center: toMapCoord(next.center),
			...(next.zoom !== undefined && next.zoom !== null
				? { zoom: next.zoom }
				: {}),
			bearing: next.bearing ?? 0,
			pitch: next.pitch ?? 0,
			duration: 1500,
			essential: true,
		});
		// Registered after flyTo: interrupting a prior flight fires a synchronous
		// moveend that must not consume this listener, and stale listeners from
		// superseded flights must not clear the flag mid-flight.
		map.once("moveend", () => {
			if (flightId === flightIdRef.current) {
				programmaticMoveRef.current = false;
			}
		});
	}, [viewportKey]);

	const handleViewportChange = useCallback(
		(vp: {
			center: [number, number];
			zoom: number;
			bearing: number;
			pitch: number;
		}) => {
			// Programmatic flights fire `move` per animation frame — those are
			// not user interactions.
			if (programmaticMoveRef.current) return;
			const context = {
				center: { latitude: vp.center[1], longitude: vp.center[0] },
				zoom: vp.zoom,
				bearing: vp.bearing,
				pitch: vp.pitch,
			};
			onAction?.({
				type: "userAction",
				name: "viewportChange",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context,
			});
			void triggerEvent("viewportChange", component, context, {
				legacyFallback: false,
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const handleMarkerClick = useCallback(
		(marker: GeoMapMarkerDef) => {
			if (marker.popup) {
				setActivePopupId((prev) => (prev === marker.id ? null : marker.id));
			}
			onAction?.({
				type: "userAction",
				name: "markerClick",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: {
					markerId: marker.id,
					coordinate: marker.coordinate,
				},
			});
			void triggerEvent("markerClick", component, {
				event: "markerClick",
				markerId: marker.id,
				coordinate: marker.coordinate,
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const handleMarkerDragEnd = useCallback(
		(markerId: string, lngLat: { lng: number; lat: number }) => {
			onAction?.({
				type: "userAction",
				name: "markerDragEnd",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: {
					markerId,
					coordinate: { latitude: lngLat.lat, longitude: lngLat.lng },
				},
			});
			void triggerEvent("markerDragEnd", component, {
				event: "markerDragEnd",
				markerId,
				coordinate: { latitude: lngLat.lat, longitude: lngLat.lng },
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const handleRouteClick = useCallback(
		(routeId: string) => {
			onAction?.({
				type: "userAction",
				name: "routeClick",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { routeId },
			});
			void triggerEvent("routeClick", component, {
				event: "routeClick",
				routeId,
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const handleLocate = useCallback(
		(coords: { longitude: number; latitude: number }) => {
			onAction?.({
				type: "userAction",
				name: "locate",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: {
					coordinate: {
						latitude: coords.latitude,
						longitude: coords.longitude,
					},
				},
			});
			void triggerEvent("locate", component, {
				event: "locate",
				coordinate: {
					latitude: coords.latitude,
					longitude: coords.longitude,
				},
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const validControlPosition = (
		["top-left", "top-right", "bottom-left", "bottom-right"] as const
	).includes(
		controlPosition as
			| "top-left"
			| "top-right"
			| "bottom-left"
			| "bottom-right",
	)
		? (controlPosition as
				| "top-left"
				| "top-right"
				| "bottom-left"
				| "bottom-right")
		: "bottom-right";

	// The map needs explicit dimensions to render.
	// We use a fixed height as default that can be overridden via style.
	return (
		<div
			className={cn("relative w-full", resolveStyle(style))}
			style={{
				height: "300px",
				...resolveInlineStyle(style),
			}}
		>
			<Map
				ref={mapRef}
				className="w-full h-full rounded-lg overflow-hidden border border-border/50 shadow-sm"
				center={viewport?.center ? toMapCoord(viewport.center) : [0, 20]}
				zoom={viewport?.zoom ?? 2}
				onViewportChange={handleViewportChange}
				interactive={interactive}
				// Keep the WebGL buffer readable so chat-widget snapshots don't
				// rasterize the map as a blank canvas.
				canvasContextAttributes={{ preserveDrawingBuffer: true }}
			>
				{showControls && (
					<MapControls
						position={validControlPosition}
						showZoom={showZoom}
						showCompass={showCompass}
						showLocate={showLocate}
						showFullscreen={showFullscreen}
						onLocate={handleLocate}
					/>
				)}

				{routes?.map((route) =>
					route.coordinates.length >= 2 ? (
						<MapRoute
							key={route.id}
							id={route.id}
							coordinates={routeToLngLat(route.coordinates)}
							color={route.color ?? "#4285F4"}
							width={route.width ?? 3}
							opacity={route.opacity ?? 0.8}
							dashArray={route.dashArray}
							onClick={() => handleRouteClick(route.id)}
						/>
					) : null,
				)}

				{markers?.map((marker) => (
					<MapMarker
						key={marker.id}
						longitude={marker.coordinate.longitude}
						latitude={marker.coordinate.latitude}
						draggable={marker.draggable ?? false}
						onClick={() => handleMarkerClick(marker)}
						onDragEnd={(lngLat) => handleMarkerDragEnd(marker.id, lngLat)}
					>
						<MarkerContent>
							<MarkerDot color={marker.color} />
						</MarkerContent>
						{marker.label && <MarkerLabel>{marker.label}</MarkerLabel>}
					</MapMarker>
				))}

				{markers
					?.filter((m) => m.popup && activePopupId === m.id)
					.map((marker) => (
						<MapPopup
							key={`popup-${marker.id}`}
							longitude={marker.coordinate.longitude}
							latitude={marker.coordinate.latitude}
							closeButton
							onClose={() => setActivePopupId(null)}
						>
							<p className="text-sm">{marker.popup}</p>
						</MapPopup>
					))}
			</Map>
		</div>
	);
}
