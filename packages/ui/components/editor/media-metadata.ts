import type { TElement } from "platejs";

export interface MediaMetadata {
	alt?: string;
	credit?: string;
	license?: string;
	rightsExpiresAt?: number;
	focalPoint?: { x: number; y: number };
}

export function mediaObjectPosition(
	element: MediaMetadata,
): string | undefined {
	const point = element.focalPoint;
	if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y))
		return undefined;
	return `${Math.max(0, Math.min(100, point.x))}% ${Math.max(0, Math.min(100, point.y))}%`;
}

export function mediaAttribution(element: MediaMetadata): string {
	return [element.credit, element.license].filter(Boolean).join(" · ");
}

export type EditorialMediaElement = TElement & MediaMetadata;
