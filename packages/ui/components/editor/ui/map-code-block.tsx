"use client";

import { useTranslation } from "@flow-like/locales";
import { Suspense, lazy, useMemo } from "react";
import { cn } from "../../../lib/utils";

interface MapCodeBlockProps {
	content: string;
	className?: string;
}

interface MapMarker {
	lat: number;
	lng: number;
	label?: string;
	color?: string;
}

interface MapConfig {
	center?: [number, number];
	zoom?: number;
	title?: string;
	type: "markers" | "route";
	mode?: "driving" | "walking" | "cycling";
	markers: MapMarker[];
}

const NAMED_COLORS: Record<string, string> = {
	red: "#EF4444",
	blue: "#3B82F6",
	green: "#22C55E",
	orange: "#F97316",
	purple: "#8B5CF6",
	yellow: "#EAB308",
	pink: "#EC4899",
	cyan: "#06B6D4",
};

function resolveColor(color?: string): string {
	if (!color) return "#3B82F6";
	return NAMED_COLORS[color.toLowerCase()] || color;
}

function parseCSVSection(csv: string): MapMarker[] {
	const lines = csv
		.trim()
		.split("\n")
		.map((l) => l.trim())
		.filter(Boolean);
	if (lines.length < 2) return [];

	const headers = lines[0].split(",").map((h) => h.trim().toLowerCase());
	const latIdx = headers.indexOf("lat");
	const lngIdx = headers.indexOf("lng");
	if (latIdx === -1 || lngIdx === -1) return [];

	const labelIdx = headers.indexOf("label");
	const colorIdx = headers.indexOf("color");

	return lines
		.slice(1)
		.map((line) => {
			const cells = line.split(",").map((c) => c.trim());
			return {
				lat: Number.parseFloat(cells[latIdx]),
				lng: Number.parseFloat(cells[lngIdx]),
				label: labelIdx !== -1 ? cells[labelIdx] : undefined,
				color: colorIdx !== -1 ? cells[colorIdx] : undefined,
			};
		})
		.filter((m) => !Number.isNaN(m.lat) && !Number.isNaN(m.lng));
}

function parseMapContent(raw: string): MapConfig {
	const trimmed = raw.trim();

	// JSON mode
	if (trimmed.startsWith("{")) {
		try {
			const json = JSON.parse(trimmed);
			return {
				center: json.center,
				zoom: json.zoom,
				title: json.title,
				type: json.type || "markers",
				mode: json.mode,
				markers: (json.markers || []).map((m: any) => ({
					lat: m.lat,
					lng: m.lng,
					label: m.label,
					color: m.color,
				})),
			};
		} catch {
			return { type: "markers", markers: [] };
		}
	}

	// CSV or key-value mode
	const config: MapConfig = { type: "markers", markers: [] };
	const separatorIndex = trimmed.indexOf("\n---\n");

	if (separatorIndex !== -1) {
		// Has CSV section
		const headerSection = trimmed.slice(0, separatorIndex);
		const csvSection = trimmed.slice(separatorIndex + 5);
		applyKeyValues(config, headerSection);
		config.markers = parseCSVSection(csvSection);
	} else {
		// Pure key-value (single marker)
		applyKeyValues(config, trimmed);
		const kv = parseKeyValues(trimmed);
		if (kv.lat !== undefined && kv.lng !== undefined) {
			config.markers = [
				{
					lat: Number.parseFloat(kv.lat),
					lng: Number.parseFloat(kv.lng),
					label: kv.label,
					color: kv.color,
				},
			];
		}
	}

	return config;
}

function parseKeyValues(text: string): Record<string, string> {
	const result: Record<string, string> = {};
	for (const line of text.split("\n")) {
		const colonIndex = line.indexOf(":");
		if (colonIndex === -1) continue;
		const key = line.slice(0, colonIndex).trim().toLowerCase();
		const value = line.slice(colonIndex + 1).trim();
		result[key] = value;
	}
	return result;
}

function applyKeyValues(config: MapConfig, text: string): void {
	const kv = parseKeyValues(text);
	if (kv.zoom) config.zoom = Number.parseInt(kv.zoom, 10);
	if (kv.title) config.title = kv.title;
	if (kv.type === "route") config.type = "route";
	if (kv.mode) config.mode = kv.mode as MapConfig["mode"];
	if (kv.center) {
		try {
			const parsed = JSON.parse(kv.center);
			if (Array.isArray(parsed) && parsed.length === 2) {
				config.center = parsed as [number, number];
			}
		} catch {
			// ignore
		}
	}
}

function calculateBounds(markers: MapMarker[]): {
	center: [number, number];
	zoom: number;
} {
	if (markers.length === 0) return { center: [0, 0], zoom: 2 };
	if (markers.length === 1)
		return { center: [markers[0].lat, markers[0].lng], zoom: 14 };

	const lats = markers.map((m) => m.lat);
	const lngs = markers.map((m) => m.lng);
	const minLat = Math.min(...lats);
	const maxLat = Math.max(...lats);
	const minLng = Math.min(...lngs);
	const maxLng = Math.max(...lngs);

	const center: [number, number] = [
		(minLat + maxLat) / 2,
		(minLng + maxLng) / 2,
	];

	const latSpan = maxLat - minLat;
	const lngSpan = maxLng - minLng;
	const maxSpan = Math.max(latSpan, lngSpan);

	let zoom = 12;
	if (maxSpan > 100) zoom = 2;
	else if (maxSpan > 40) zoom = 3;
	else if (maxSpan > 20) zoom = 4;
	else if (maxSpan > 10) zoom = 5;
	else if (maxSpan > 5) zoom = 6;
	else if (maxSpan > 2) zoom = 7;
	else if (maxSpan > 1) zoom = 9;
	else if (maxSpan > 0.5) zoom = 10;
	else if (maxSpan > 0.1) zoom = 12;
	else zoom = 14;

	return { center, zoom };
}

function MapFallback() {
	const { t } = useTranslation("common");
	return (
		<div className="flex items-center justify-center h-80 bg-muted/20 rounded-lg animate-pulse">
			<span className="text-muted-foreground text-sm">{t('loadingMap', 'Loading map...')}</span>
		</div>
	);
}

function MapErrorFallback({ markers }: { markers: MapMarker[] }) {
	const { t } = useTranslation("common");
	return (
		<div className="rounded-md border border-border/50 bg-muted/20 p-4 text-sm">
			<p className="font-medium mb-2">{t('mapLocations', 'Map Locations')}</p>
			<ul className="space-y-1">
				{markers.map((marker, i) => (
					<li
						key={`${marker.lat}-${marker.lng}-${i}`}
						className="text-muted-foreground"
					>
						{marker.label || t('pointVal', 'Point {{val}}', { val: i + 1 })}: {marker.lat.toFixed(4)},{" "}
						{marker.lng.toFixed(4)}
					</li>
				))}
			</ul>
		</div>
	);
}

const MapLibreMap = lazy(() => import("./map-libre-renderer"));

export function MapCodeBlock({ content, className }: MapCodeBlockProps) {
	const { t } = useTranslation("common");
	const config = useMemo(() => parseMapContent(content), [content]);

	const hasExplicitZoom = config.zoom != null;

	const { center, zoom } = useMemo(() => {
		if (config.center)
			return { center: config.center, zoom: config.zoom ?? 12 };
		const bounds = calculateBounds(config.markers);
		return { center: bounds.center, zoom: config.zoom ?? bounds.zoom };
	}, [config]);

	if (config.markers.length === 0) {
		return (
			<div
				className={cn(
					"p-4 text-sm text-muted-foreground rounded-md border border-border/50",
					className,
				)}
			>
				{t('noMapMarkersFoundProvideLatlngCoordinates', 'No map markers found. Provide lat/lng coordinates.')}
			</div>
		);
	}

	return (
		<div className={cn("my-2", className)}>
			{config.title && (
				<p className="text-sm font-medium mb-2">{config.title}</p>
			)}
			<Suspense fallback={<MapFallback />}>
				<MapLibreMap
					center={center}
					zoom={zoom}
					markers={config.markers.map((m) => ({
						...m,
						color: resolveColor(m.color),
					}))}
					isRoute={config.type === "route"}
					fitBounds={!hasExplicitZoom && config.markers.length > 1}
				/>
			</Suspense>
		</div>
	);
}

export default MapCodeBlock;
