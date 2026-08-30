"use client";

import { useTranslation } from "@flow-like/locales";
import { Monitor, RotateCcw, Smartphone, Tablet } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib";
import { Button } from "../ui/button";
import { PortalContainerProvider } from "../ui/portal-container";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";

interface DevicePreset {
	name: string;
	width: number;
	height: number;
	icon: typeof Monitor;
}

const DEVICE_PRESETS: DevicePreset[] = [
	{ name: "Desktop", width: 1440, height: 900, icon: Monitor },
	{ name: "Laptop", width: 1280, height: 800, icon: Monitor },
	{ name: "Tablet", width: 768, height: 1024, icon: Tablet },
	{ name: "Mobile", width: 375, height: 812, icon: Smartphone },
	{ name: "Mobile Small", width: 320, height: 568, icon: Smartphone },
];

const BREAKPOINTS = {
	sm: 640,
	md: 768,
	lg: 1024,
	xl: 1280,
	"2xl": 1536,
};

const FRAME_STYLE_MARK = "data-responsive-preview-style";
const FRAME_MOUNT_ID = "responsive-preview-root";

function copyParentStyles(doc: Document) {
	for (const node of Array.from(
		doc.head.querySelectorAll(`[${FRAME_STYLE_MARK}]`),
	)) {
		node.remove();
	}
	const sources = document.head.querySelectorAll(
		'style, link[rel="stylesheet"]',
	);
	for (const source of Array.from(sources)) {
		let copy: HTMLElement;
		if (source instanceof HTMLLinkElement) {
			const link = doc.createElement("link");
			link.rel = "stylesheet";
			link.href = source.href;
			copy = link;
		} else {
			const style = doc.createElement("style");
			style.textContent = source.textContent;
			copy = style;
		}
		copy.setAttribute(FRAME_STYLE_MARK, "true");
		doc.head.appendChild(copy);
	}
}

function mirrorTheme(doc: Document) {
	doc.documentElement.className = document.documentElement.className;
	doc.documentElement.style.cssText = document.documentElement.style.cssText;
	const dataTheme = document.documentElement.getAttribute("data-theme");
	if (dataTheme === null) {
		doc.documentElement.removeAttribute("data-theme");
	} else {
		doc.documentElement.setAttribute("data-theme", dataTheme);
	}
	doc.body.className = document.body.className;
}

interface PreviewFrameProps {
	width: number;
	height: number;
	scale: number;
	children: React.ReactNode;
}

/**
 * Renders children into a same-origin iframe so viewport media queries
 * (Tailwind `sm:`/`md:`/… and the `a2ui-*` responsive classes) evaluate
 * against the simulated device width instead of the host window.
 */
function PreviewFrame({ width, height, scale, children }: PreviewFrameProps) {
	const { t } = useTranslation("flow");
	const iframeRef = useRef<HTMLIFrameElement>(null);
	const [mountNode, setMountNode] = useState<HTMLElement | null>(null);

	useEffect(() => {
		const iframe = iframeRef.current;
		if (!iframe) return;

		let headObserver: MutationObserver | null = null;
		let themeObserver: MutationObserver | null = null;

		const initialize = () => {
			const doc = iframe.contentDocument;
			if (!doc?.body) return;

			copyParentStyles(doc);
			mirrorTheme(doc);
			doc.documentElement.style.overscrollBehavior = "none";

			let mount = doc.getElementById(FRAME_MOUNT_ID);
			if (!mount) {
				mount = doc.createElement("div");
				mount.id = FRAME_MOUNT_ID;
				mount.style.height = "100%";
				doc.body.appendChild(mount);
			}

			headObserver?.disconnect();
			headObserver = new MutationObserver(() => copyParentStyles(doc));
			headObserver.observe(document.head, {
				childList: true,
				subtree: true,
				characterData: true,
			});

			themeObserver?.disconnect();
			themeObserver = new MutationObserver(() => mirrorTheme(doc));
			themeObserver.observe(document.documentElement, { attributes: true });
			themeObserver.observe(document.body, {
				attributes: true,
				attributeFilter: ["class"],
			});

			setMountNode(mount);
		};

		initialize();
		// Some engines replace the initial about:blank document after mount.
		iframe.addEventListener("load", initialize);
		return () => {
			iframe.removeEventListener("load", initialize);
			headObserver?.disconnect();
			themeObserver?.disconnect();
		};
	}, []);

	// Clicking anything in the frame moves focus into it, which fires `blur` on the host
	// window — and Radix closes every open overlay on that event. A select opened in the
	// preview therefore shut before the user could pick an option. Focus landing in the
	// preview is not the app losing focus, so that one event is swallowed before any other
	// listener sees it. Element blurs (target is the element, not the window) pass through.
	useEffect(() => {
		const swallowFocusHandoff = (event: Event) => {
			if (
				event.target === window &&
				document.activeElement === iframeRef.current
			) {
				event.stopImmediatePropagation();
			}
		};
		window.addEventListener("blur", swallowFocusHandoff, true);
		return () => window.removeEventListener("blur", swallowFocusHandoff, true);
	}, []);

	return (
		<>
			<iframe
				ref={iframeRef}
				title={t("responsivePreview", "Responsive preview")}
				className="bg-background"
				style={{
					width,
					height,
					border: 0,
					display: "block",
					transform: `scale(${scale})`,
					transformOrigin: "top left",
				}}
			/>
			{mountNode
				? createPortal(
						<PortalContainerProvider container={mountNode}>
							{children}
						</PortalContainerProvider>,
						mountNode,
					)
				: null}
		</>
	);
}

export interface ResponsivePreviewProps {
	className?: string;
	children: React.ReactNode;
}

export function ResponsivePreview({
	className,
	children,
}: ResponsivePreviewProps) {
	const { t } = useTranslation("flow");
	const [selectedDevice, setSelectedDevice] = useState(DEVICE_PRESETS[0]);
	const [orientation, setOrientation] = useState<"portrait" | "landscape">(
		"landscape",
	);
	const [availableSize, setAvailableSize] = useState<{
		width: number;
		height: number;
	} | null>(null);
	const previewAreaRef = useRef<HTMLDivElement>(null);

	const displayWidth =
		orientation === "portrait" ? selectedDevice.height : selectedDevice.width;
	const displayHeight =
		orientation === "portrait" ? selectedDevice.width : selectedDevice.height;

	const activeBreakpoint = (() => {
		if (displayWidth >= BREAKPOINTS["2xl"]) return "2xl";
		if (displayWidth >= BREAKPOINTS.xl) return "xl";
		if (displayWidth >= BREAKPOINTS.lg) return "lg";
		if (displayWidth >= BREAKPOINTS.md) return "md";
		if (displayWidth >= BREAKPOINTS.sm) return "sm";
		return "xs";
	})();

	useEffect(() => {
		const el = previewAreaRef.current;
		if (!el) return;
		const observer = new ResizeObserver((entries) => {
			const rect = entries[0]?.contentRect;
			if (rect) setAvailableSize({ width: rect.width, height: rect.height });
		});
		observer.observe(el);
		return () => observer.disconnect();
	}, []);

	const scale = availableSize
		? Math.min(
				1,
				availableSize.width / displayWidth,
				availableSize.height / displayHeight,
			)
		: null;

	const handleDeviceSelect = useCallback((deviceName: string) => {
		const device = DEVICE_PRESETS.find((d) => d.name === deviceName);
		if (device) {
			setSelectedDevice(device);
		}
	}, []);

	const toggleOrientation = useCallback(() => {
		setOrientation((prev) => (prev === "portrait" ? "landscape" : "portrait"));
	}, []);

	return (
		<div className={cn("flex flex-col h-full min-w-0", className)}>
			{/* Toolbar */}
			<div className="flex items-center justify-between gap-4 p-2 border-b bg-background min-w-0 overflow-hidden">
				{/* Device selector */}
				<div className="flex items-center gap-2 shrink-0">
					<Select
						value={selectedDevice.name}
						onValueChange={handleDeviceSelect}
					>
						<SelectTrigger className="w-36 h-8">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{DEVICE_PRESETS.map((device) => {
								const Icon = device.icon;
								return (
									<SelectItem key={device.name} value={device.name}>
										<div className="flex items-center gap-2">
											<Icon className="h-4 w-4" />
											{device.name}
										</div>
									</SelectItem>
								);
							})}
						</SelectContent>
					</Select>

					<Button
						variant="ghost"
						size="sm"
						onClick={toggleOrientation}
						title={t("toggleOrientation", "Toggle orientation")}
					>
						<RotateCcw className="h-4 w-4" />
					</Button>
				</div>

				{/* Size display */}
				<div className="flex items-center gap-2 text-sm text-muted-foreground whitespace-nowrap min-w-0">
					<span>{`${displayWidth} × ${displayHeight}`}</span>
					{scale !== null && scale < 1 && (
						<span className="text-xs">{Math.round(scale * 100)}%</span>
					)}
					<span className="px-1.5 py-0.5 rounded bg-muted text-xs font-medium">
						{activeBreakpoint}
					</span>
				</div>

				{/* Quick device buttons */}
				<div className="flex items-center gap-1 shrink-0">
					{DEVICE_PRESETS.slice(0, 4).map((device) => {
						const Icon = device.icon;
						return (
							<Button
								key={device.name}
								variant={
									selectedDevice.name === device.name ? "secondary" : "ghost"
								}
								size="sm"
								onClick={() => setSelectedDevice(device)}
								title={device.name}
							>
								<Icon className="h-4 w-4" />
							</Button>
						);
					})}
				</div>
			</div>

			{/* Preview area */}
			<div
				ref={previewAreaRef}
				className="flex-1 flex items-center justify-center bg-muted/30 overflow-hidden p-4 min-w-0 min-h-0"
			>
				{scale !== null && (
					<div
						className="bg-background shadow-lg rounded-lg overflow-hidden shrink-0"
						style={{
							width: Math.round(displayWidth * scale),
							height: Math.round(displayHeight * scale),
						}}
					>
						<PreviewFrame
							width={displayWidth}
							height={displayHeight}
							scale={scale}
						>
							{children}
						</PreviewFrame>
					</div>
				)}
			</div>

			{/* Breakpoint indicator bar */}
			<div className="flex items-center flex-wrap gap-y-1 min-h-8 py-1 px-4 border-t bg-background text-xs min-w-0">
				<div className="flex items-center flex-wrap gap-2 flex-1 min-w-0">
					{Object.entries(BREAKPOINTS).map(([name, width]) => (
						<div
							key={name}
							className={cn(
								"px-2 py-0.5 rounded transition-colors whitespace-nowrap",
								displayWidth >= width
									? "bg-primary/10 text-primary font-medium"
									: "text-muted-foreground",
							)}
						>
							{t("nameWidthpx", "{{name}} ({{width}}px)", { name, width })}
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

export interface SideBySidePreviewProps {
	className?: string;
	children: React.ReactNode;
}

export function SideBySidePreview({
	className,
	children,
}: SideBySidePreviewProps) {
	const devices = [
		DEVICE_PRESETS[0], // Desktop
		DEVICE_PRESETS[3], // Mobile
	];

	return (
		<div
			className={cn(
				"flex h-full gap-4 p-4 bg-muted/30 overflow-auto",
				className,
			)}
		>
			{devices.map((device) => {
				const Icon = device.icon;
				return (
					<div key={device.name} className="flex flex-col gap-2">
						<div className="flex items-center gap-2 text-sm text-muted-foreground">
							<Icon className="h-4 w-4" />
							<span>{device.name}</span>
							<span className="text-xs">{`(${device.width}×${device.height})`}</span>
						</div>
						<div
							className="bg-background shadow-lg rounded-lg overflow-hidden shrink-0"
							style={{
								width: device.width * 0.5,
								height: device.height * 0.5,
							}}
						>
							<PreviewFrame
								width={device.width}
								height={device.height}
								scale={0.5}
							>
								{children}
							</PreviewFrame>
						</div>
					</div>
				);
			})}
		</div>
	);
}
