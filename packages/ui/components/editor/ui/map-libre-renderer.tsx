"use client";

import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { useTheme } from "next-themes";
import { useEffect, useRef } from "react";

interface MapMarker {
	lat: number;
	lng: number;
	label?: string;
	color?: string;
}

interface MapLibreRendererProps {
	center: [number, number];
	zoom: number;
	markers: MapMarker[];
	isRoute?: boolean;
	fitBounds?: boolean;
}

function buildStyle(isDark: boolean): maplibregl.StyleSpecification {
	return isDark
		? {
				version: 8,
				sources: {
					carto: {
						type: "raster",
						tiles: [
							"https://a.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png",
							"https://b.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png",
							"https://c.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png",
						],
						tileSize: 256,
						attribution:
							'&copy; <a href="https://www.openstreetmap.org/copyright">OSM</a> &copy; <a href="https://carto.com/">CARTO</a>',
					},
				},
				layers: [
					{
						id: "carto-tiles",
						type: "raster",
						source: "carto",
						minzoom: 0,
						maxzoom: 20,
					},
				],
			}
		: {
				version: 8,
				sources: {
					carto: {
						type: "raster",
						tiles: [
							"https://a.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}@2x.png",
							"https://b.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}@2x.png",
							"https://c.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}@2x.png",
						],
						tileSize: 256,
						attribution:
							'&copy; <a href="https://www.openstreetmap.org/copyright">OSM</a> &copy; <a href="https://carto.com/">CARTO</a>',
					},
				},
				layers: [
					{
						id: "carto-tiles",
						type: "raster",
						source: "carto",
						minzoom: 0,
						maxzoom: 20,
					},
				],
			};
}

function createMarkerElement(
	color: string,
	label?: string,
	isDark = false,
): HTMLDivElement {
	const wrapper = document.createElement("div");
	wrapper.style.display = "flex";
	wrapper.style.flexDirection = "column";
	wrapper.style.alignItems = "center";
	wrapper.style.cursor = "pointer";

	const pin = document.createElement("div");
	pin.style.width = "28px";
	pin.style.height = "28px";
	pin.style.borderRadius = "50% 50% 50% 0";
	pin.style.backgroundColor = color;
	pin.style.transform = "rotate(-45deg)";
	pin.style.border = `2.5px solid ${isDark ? "rgba(255,255,255,0.9)" : "white"}`;
	pin.style.boxShadow = isDark
		? "0 2px 12px rgba(0,0,0,0.6)"
		: "0 2px 8px rgba(0,0,0,0.35)";
	pin.style.position = "relative";

	const dot = document.createElement("div");
	dot.style.width = "8px";
	dot.style.height = "8px";
	dot.style.borderRadius = "50%";
	dot.style.backgroundColor = "white";
	dot.style.position = "absolute";
	dot.style.top = "50%";
	dot.style.left = "50%";
	dot.style.transform = "translate(-50%, -50%) rotate(45deg)";
	pin.appendChild(dot);
	wrapper.appendChild(pin);

	if (label) {
		const labelEl = document.createElement("div");
		labelEl.textContent = label;
		labelEl.style.marginTop = "4px";
		labelEl.style.padding = "2px 6px";
		labelEl.style.borderRadius = "4px";
		labelEl.style.backgroundColor = isDark
			? "rgba(0,0,0,0.85)"
			: "rgba(0,0,0,0.7)";
		labelEl.style.color = "white";
		labelEl.style.fontSize = "11px";
		labelEl.style.fontWeight = "500";
		labelEl.style.whiteSpace = "nowrap";
		labelEl.style.maxWidth = "150px";
		labelEl.style.overflow = "hidden";
		labelEl.style.textOverflow = "ellipsis";
		wrapper.appendChild(labelEl);
	}

	return wrapper;
}

export function MapLibreRenderer({
	center,
	zoom,
	markers,
	isRoute = false,
	fitBounds = false,
}: MapLibreRendererProps) {
	const containerRef = useRef<HTMLDivElement>(null);
	const mapRef = useRef<maplibregl.Map | null>(null);
	const { resolvedTheme } = useTheme();
	const isDark = resolvedTheme === "dark";

	useEffect(() => {
		if (!containerRef.current) return;

		const map = new maplibregl.Map({
			container: containerRef.current,
			style: buildStyle(isDark),
			center: [center[1], center[0]],
			zoom: fitBounds ? 2 : zoom,
			attributionControl: false,
		});

		map.addControl(
			new maplibregl.NavigationControl({ showCompass: false }),
			"top-right",
		);
		map.addControl(
			new maplibregl.AttributionControl({ compact: true }),
			"bottom-right",
		);

		map.on("load", () => {
			for (const marker of markers) {
				const el = createMarkerElement(
					marker.color || "#3B82F6",
					marker.label,
					isDark,
				);
				const m = new maplibregl.Marker({
					element: el,
					anchor: "bottom",
				}).setLngLat([marker.lng, marker.lat]);

				if (marker.label) {
					m.setPopup(
						new maplibregl.Popup({ offset: 25, closeButton: false }).setText(
							marker.label,
						),
					);
				}

				m.addTo(map);
			}

			// fitBounds: automatically zoom/pan to enclose all markers
			if (fitBounds && markers.length > 0) {
				const bounds = new maplibregl.LngLatBounds();
				for (const m of markers) {
					bounds.extend([m.lng, m.lat]);
				}
				map.fitBounds(bounds, {
					padding: { top: 50, bottom: 50, left: 50, right: 50 },
					maxZoom: 15,
					duration: 0,
				});
			}

			if (isRoute && markers.length >= 2) {
				const coordinates = markers.map(
					(m) => [m.lng, m.lat] as [number, number],
				);
				map.addSource("route", {
					type: "geojson",
					data: {
						type: "Feature",
						properties: {},
						geometry: { type: "LineString", coordinates },
					},
				});
				map.addLayer({
					id: "route-line-bg",
					type: "line",
					source: "route",
					layout: { "line-join": "round", "line-cap": "round" },
					paint: {
						"line-color": isDark ? "#60a5fa" : "#1d4ed8",
						"line-width": 6,
						"line-opacity": 0.25,
					},
				});
				map.addLayer({
					id: "route-line",
					type: "line",
					source: "route",
					layout: { "line-join": "round", "line-cap": "round" },
					paint: {
						"line-color": "#3B82F6",
						"line-width": 3,
					},
				});
			}
		});

		mapRef.current = map;
		return () => {
			map.remove();
			mapRef.current = null;
		};
	}, [center, zoom, markers, isRoute, isDark, fitBounds]);

	return (
		<div
			ref={containerRef}
			className="h-80 w-full rounded-lg overflow-hidden border border-border/30"
		/>
	);
}

export default MapLibreRenderer;
