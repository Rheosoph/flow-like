"use client";

import { cn } from "../../../lib/utils";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { BoundValue, MapMarkerDef, MiniMapComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIMiniMap({
	elementRef,
	component,
	style,
	componentId,
}: ComponentProps<MiniMapComponent>) {
	const { url: mapImage } = useAssetUrl(
		useResolved<string>(component.mapImage),
	);
	const width = useResolved<string>(component.width) ?? "200px";
	const height = useResolved<string>(component.height) ?? "200px";
	const markers = useResolved<MapMarkerDef[]>(component.markers) ?? [];
	const playerX = useResolved<number>(component.playerX);
	const playerY = useResolved<number>(component.playerY);
	const playerRotation = useResolved<number>(component.playerRotation) ?? 0;
	const triggerEvent = useComponentEventTrigger(componentId);

	const handleMarkerClick = (marker: MapMarkerDef) => {
		void triggerEvent("markerClick", component, {
			markerId: marker.id,
			markerX: marker.x,
			markerY: marker.y,
		});
	};

	return (
		<div
			ref={elementRef}
			className={cn(
				"relative overflow-hidden rounded-lg border bg-muted",
				resolveStyle(style),
			)}
			style={{ width, height, ...resolveInlineStyle(style) }}
		>
			{mapImage && (
				<img
					src={mapImage}
					alt="Map"
					className="absolute inset-0 w-full h-full object-cover"
				/>
			)}

			{markers.map((marker) => {
				const markerColor = marker.color
					? useResolved<string>(marker.color)
					: "#ef4444";
				const markerLabel = marker.label
					? useResolved<string>(marker.label)
					: undefined;
				return (
					<button
						key={marker.id}
						type="button"
						className="absolute w-3 h-3 -translate-x-1/2 -translate-y-1/2 rounded-full"
						style={{
							left: `${marker.x}%`,
							top: `${marker.y}%`,
							backgroundColor: markerColor,
						}}
						onClick={() => handleMarkerClick(marker)}
						title={markerLabel}
					/>
				);
			})}

			{playerX !== undefined && playerY !== undefined && (
				<div
					className="absolute w-4 h-4 -translate-x-1/2 -translate-y-1/2"
					style={{
						left: `${playerX}%`,
						top: `${playerY}%`,
						transform: `translate(-50%, -50%) rotate(${playerRotation}deg)`,
					}}
				>
					<svg
						viewBox="0 0 24 24"
						className="w-full h-full text-primary fill-current"
					>
						<path d="M12 2L19 21L12 17L5 21L12 2Z" />
					</svg>
				</div>
			)}
		</div>
	);
}
