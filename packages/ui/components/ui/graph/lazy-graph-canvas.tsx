"use client";

import { useTranslation } from "@flow-like/locales";
import { MonitorX } from "lucide-react";
import { Suspense, lazy } from "react";
import { useWebGlSupport } from "../../../hooks/use-webgl-support";
import { cn } from "../../../lib/utils";
import type { GraphCanvasProps } from "./graph-canvas";

// `@sigma/node-image` calls `getContext("webgl").getParameter` while its module
// evaluates, so without WebGL importing the canvas throws and takes the page down.
const SigmaGraphCanvas = lazy(() =>
	import("./graph-canvas").then((module) => ({ default: module.GraphCanvas })),
);

function WebGlUnavailable({ className }: { className?: string }) {
	const { t } = useTranslation("common");
	return (
		<div
			className={cn(
				"relative flex h-full w-full flex-col items-center justify-center gap-2 overflow-hidden p-6 text-center",
				className,
			)}
		>
			<MonitorX className="h-6 w-6 text-muted-foreground" />
			<p className="text-sm font-medium">
				{t("graphViewUnavailable", "Graph view unavailable")}
			</p>
			<p className="max-w-sm text-xs text-muted-foreground">
				{t(
					"graphViewNeedsWebGl",
					"Drawing the graph needs WebGL, which this browser has turned off or does not support. Turn on hardware acceleration or use another browser.",
				)}
			</p>
		</div>
	);
}

export function GraphCanvas(props: GraphCanvasProps) {
	const webGl = useWebGlSupport("webgl");

	if (webGl === false) return <WebGlUnavailable className={props.className} />;

	const placeholder = (
		<div
			className={cn("relative h-full w-full overflow-hidden", props.className)}
		/>
	);
	if (webGl === null) return placeholder;

	return (
		<Suspense fallback={placeholder}>
			<SigmaGraphCanvas {...props} />
		</Suspense>
	);
}
