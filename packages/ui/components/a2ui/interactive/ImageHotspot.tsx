"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { BoundValue, Hotspot, ImageHotspotComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIImageHotspot({
	component,
	style,
	onAction,
	surfaceId,
	componentId,
}: ComponentProps<ImageHotspotComponent>) {
	const triggerEvent = useComponentEventTrigger(componentId);
	const containerRef = useRef<HTMLDivElement>(null);
	const imageRef = useRef<HTMLImageElement>(null);

	const { url: src } = useAssetUrl(useResolved<string>(component.src));
	const alt = useResolved<string>(component.alt) ?? "Interactive image";
	const rawHotspots = useResolved<Hotspot[]>(component.hotspots) ?? [];
	const showMarkers = useResolved<boolean>(component.showMarkers) ?? true;
	const markerStyle = useResolved<string>(component.markerStyle) ?? "pulse";
	const fit = useResolved<string>(component.fit) ?? "contain";
	const normalized = useResolved<boolean>(component.normalized) ?? false;
	const showTooltips = useResolved<boolean>(component.showTooltips) ?? true;

	const [imageLoaded, setImageLoaded] = useState(false);
	const [imageSize, setImageSize] = useState({ width: 0, height: 0 });
	const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });
	const [hoveredHotspot, setHoveredHotspot] = useState<string | null>(null);
	const [activeHotspot, setActiveHotspot] = useState<string | null>(null);

	// Parse hotspots
	const hotspots = useMemo((): Hotspot[] => {
		if (!rawHotspots) return [];
		if (typeof rawHotspots === "string") {
			try {
				return JSON.parse(rawHotspots);
			} catch {
				return [];
			}
		}
		return Array.isArray(rawHotspots) ? rawHotspots : [];
	}, [rawHotspots]);

	const handleImageLoad = useCallback(() => {
		if (imageRef.current) {
			setImageSize({
				width: imageRef.current.naturalWidth,
				height: imageRef.current.naturalHeight,
			});
			setImageLoaded(true);
		}
	}, []);

	// Track container size
	useEffect(() => {
		if (!containerRef.current) return;
		const observer = new ResizeObserver((entries) => {
			for (const entry of entries) {
				setContainerSize({
					width: entry.contentRect.width,
					height: entry.contentRect.height,
				});
			}
		});
		observer.observe(containerRef.current);
		return () => observer.disconnect();
	}, []);

	// Calculate scaling
	const scale = useMemo(() => {
		if (!imageLoaded || !imageSize.width || !imageSize.height) {
			return { x: 1, y: 1, offsetX: 0, offsetY: 0 };
		}

		const imgAspect = imageSize.width / imageSize.height;
		const containerAspect =
			containerSize.width / (containerSize.height || containerSize.width);

		let displayWidth: number;
		let displayHeight: number;
		let offsetX = 0;
		let offsetY = 0;

		if (fit === "contain") {
			if (imgAspect > containerAspect) {
				displayWidth = containerSize.width;
				displayHeight = containerSize.width / imgAspect;
				offsetY = (containerSize.height - displayHeight) / 2;
			} else {
				displayHeight = containerSize.height || containerSize.width / imgAspect;
				displayWidth = displayHeight * imgAspect;
				offsetX = (containerSize.width - displayWidth) / 2;
			}
		} else {
			displayWidth = containerSize.width;
			displayHeight = containerSize.height || containerSize.width / imgAspect;
		}

		return {
			x: displayWidth / (normalized ? 1 : imageSize.width),
			y: displayHeight / (normalized ? 1 : imageSize.height),
			offsetX,
			offsetY,
		};
	}, [imageLoaded, imageSize, containerSize, fit, normalized]);

	const handleHotspotClick = useCallback(
		(hotspot: Hotspot) => {
			if (hotspot.disabled) return;

			setActiveHotspot(hotspot.id);
			setTimeout(() => setActiveHotspot(null), 300);

			const context = { hotspot, hotspotId: hotspot.id };
			void triggerEvent("hotspotClick", component, context);

			// Also fire hotspot-specific action if defined
			if (hotspot.action && onAction) {
				onAction({
					type: "userAction",
					name: hotspot.action,
					surfaceId,
					sourceComponentId: componentId,
					timestamp: Date.now(),
					context,
				});
			}

			const literalEvent = hotspot.action?.trim();
			if (literalEvent && literalEvent !== "hotspotClick") {
				void triggerEvent(literalEvent, component, context, {
					legacyFallback: false,
				});
			}
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const getMarkerClasses = (
		hotspot: Hotspot,
		isHovered: boolean,
		isActive: boolean,
	) => {
		const base =
			"absolute flex items-center justify-center transition-all duration-200 cursor-pointer";
		const disabled = hotspot.disabled ? "opacity-50 cursor-not-allowed" : "";
		const hovered = isHovered ? "scale-125 z-10" : "";
		const active = isActive ? "scale-90" : "";

		const styleClasses: Record<string, string> = {
			pulse: "rounded-full animate-pulse",
			dot: "rounded-full",
			ring: "rounded-full border-2",
			square: "rounded-sm",
			diamond: "rotate-45",
			none: "opacity-0",
		};

		return cn(
			base,
			disabled,
			hovered,
			active,
			styleClasses[markerStyle] ?? styleClasses.pulse,
		);
	};

	const fitClass =
		{
			contain: "object-contain",
			cover: "object-cover",
			fill: "object-fill",
		}[fit] ?? "object-contain";

	return (
		<div
			data-card-action-stop
			ref={containerRef}
			className={cn("relative", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<img
				ref={imageRef}
				src={src}
				alt={alt}
				onLoad={handleImageLoad}
				className={cn("w-full h-full", fitClass)}
			/>

			{/* Hotspot markers */}
			{imageLoaded &&
				showMarkers &&
				hotspots.map((hotspot) => {
					const isHovered = hoveredHotspot === hotspot.id;
					const isActive = activeHotspot === hotspot.id;

					const left = hotspot.x * scale.x + scale.offsetX;
					const top = hotspot.y * scale.y + scale.offsetY;
					const size = hotspot.size ?? 24;
					const color = hotspot.color ?? "#3b82f6";

					return (
						<div
							key={hotspot.id}
							className="absolute"
							style={{
								left: `${left}px`,
								top: `${top}px`,
								transform: "translate(-50%, -50%)",
							}}
						>
							{/* Marker */}
							<button
								type="button"
								onClick={() => handleHotspotClick(hotspot)}
								onMouseEnter={() => setHoveredHotspot(hotspot.id)}
								onMouseLeave={() => setHoveredHotspot(null)}
								className={getMarkerClasses(hotspot, isHovered, isActive)}
								style={{
									width: `${size}px`,
									height: `${size}px`,
									backgroundColor:
										markerStyle === "ring" ? "transparent" : `${color}cc`,
									borderColor: color,
								}}
								disabled={hotspot.disabled}
							>
								{hotspot.icon && (
									<span
										className="text-white"
										style={{ fontSize: `${size * 0.5}px` }}
									>
										{hotspot.icon}
									</span>
								)}
							</button>

							{/* Tooltip */}
							{showTooltips &&
								isHovered &&
								(hotspot.label || hotspot.description) && (
									<div
										className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-2 bg-popover text-popover-foreground rounded-md shadow-lg text-sm whitespace-nowrap z-20"
										style={{ minWidth: "100px", maxWidth: "200px" }}
									>
										{hotspot.label && (
											<div className="font-medium">{hotspot.label}</div>
										)}
										{hotspot.description && (
											<div className="text-muted-foreground text-xs mt-0.5 whitespace-normal">
												{hotspot.description}
											</div>
										)}
										{/* Arrow */}
										<div
											className="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent"
											style={{ borderTopColor: "hsl(var(--popover))" }}
										/>
									</div>
								)}
						</div>
					);
				})}
		</div>
	);
}
