"use client";

import { useCallback, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	Map,
	MapControls,
	MapMarker,
	MapPopup,
	MapRoute,
	MarkerContent,
	MarkerLabel,
} from "../../ui/map";
import { useComponentActionTrigger } from "../ActionHandler";
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
	const triggerAction = useComponentActionTrigger(componentId);
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

	const mapViewport = useMemo(() => {
		if (!viewport) return undefined;
		return {
			center: toMapCoord(viewport.center),
			zoom: viewport.zoom,
			bearing: viewport.bearing ?? 0,
			pitch: viewport.pitch ?? 0,
		};
	}, [viewport]);

	const handleViewportChange = useCallback(
		(vp: {
			center: [number, number];
			zoom: number;
			bearing: number;
			pitch: number;
		}) => {
			if (!onAction) return;
			onAction({
				type: "userAction",
				name: "viewportChange",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: {
					center: { latitude: vp.center[1], longitude: vp.center[0] },
					zoom: vp.zoom,
					bearing: vp.bearing,
					pitch: vp.pitch,
				},
			});
		},
		[onAction, surfaceId, componentId],
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
			void triggerAction(component.actions, {
				event: "markerClick",
				markerId: marker.id,
				coordinate: marker.coordinate,
			});
		},
		[component.actions, componentId, onAction, surfaceId, triggerAction],
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
			void triggerAction(component.actions, {
				event: "markerDragEnd",
				markerId,
				coordinate: { latitude: lngLat.lat, longitude: lngLat.lng },
			});
		},
		[component.actions, componentId, onAction, surfaceId, triggerAction],
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
			void triggerAction(component.actions, {
				event: "routeClick",
				routeId,
			});
		},
		[component.actions, componentId, onAction, surfaceId, triggerAction],
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
			void triggerAction(component.actions, {
				event: "locate",
				coordinate: {
					latitude: coords.latitude,
					longitude: coords.longitude,
				},
			});
		},
		[component.actions, componentId, onAction, surfaceId, triggerAction],
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
				className="w-full h-full rounded-lg overflow-hidden border border-border/50 shadow-sm"
				{...(mapViewport
					? { viewport: mapViewport, onViewportChange: handleViewportChange }
					: {
							center: viewport ? toMapCoord(viewport.center) : [0, 20],
							zoom: viewport?.zoom ?? 2,
						})}
				interactive={interactive}
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
