"use client";

import { BaseEdge } from "@xyflow/react";
import { createContext, useContext, useSyncExternalStore } from "react";
import {
	type BoardTrace,
	type WireEmphasis,
	wireEmphasis,
} from "../../lib/board-canvas/trace";
import type { SignalStore } from "../../lib/realtime/presence-signals-store";

/** The native board's pin trace, for edges that render in SVG. */
export const WireTraceContext = createContext<
	SignalStore<BoardTrace> | undefined
>(undefined);

const unsubscribed = () => () => {};

export function useWireEmphasis(edgeId: string): WireEmphasis | undefined {
	const store = useContext(WireTraceContext);
	const read = () =>
		store ? wireEmphasis(store.getSnapshot(), edgeId) : undefined;
	return useSyncExternalStore(store?.subscribe ?? unsubscribed, read, read);
}

/** Whether a trace lights the wire, rather than dimming it or passing it by. */
export const isLit = (emphasis?: WireEmphasis) =>
	!!emphasis && emphasis !== "dim";

/** The width and opacity of a traced wire's wide glow and narrow halo. */
const GLOW: Record<
	Exclude<WireEmphasis, "dim">,
	[[number, number], [number, number]]
> = {
	hover: [
		[12, 0.07],
		[6, 0.18],
	],
	focus: [
		[18, 0.12],
		[8, 0.36],
	],
	selected: [
		[24, 0.16],
		[11, 0.5],
	],
};

interface WireTraceLayerProps {
	id: string;
	path: string;
	color: string;
	emphasis?: WireEmphasis;
}

const traceStroke = (color: string, width: number, opacity: number) => ({
	stroke: color,
	strokeWidth: width,
	opacity,
	strokeLinecap: "round" as const,
	strokeLinejoin: "round" as const,
	pointerEvents: "none" as const,
});

/** The glow under a traced SVG edge, matching the native canvas trace. */
export function WireTraceGlow({
	id,
	path,
	color,
	emphasis,
}: Readonly<WireTraceLayerProps>) {
	if (!emphasis || emphasis === "dim") return null;
	const [glow, halo] = GLOW[emphasis];
	return (
		<>
			<BaseEdge
				id={`${id}-trace-glow`}
				path={path}
				interactionWidth={0}
				style={traceStroke(color, ...glow)}
			/>
			<BaseEdge
				id={`${id}-trace-halo`}
				path={path}
				interactionWidth={0}
				style={traceStroke(color, ...halo)}
			/>
		</>
	);
}

/** Lights running from source to target over a focused SVG edge; the clicked one also gets a
 * bright stripe down its middle. */
export function WireTraceFlow({
	id,
	path,
	color,
	emphasis,
}: Readonly<WireTraceLayerProps>) {
	if (emphasis !== "focus" && emphasis !== "selected") return null;
	return (
		<>
			{emphasis === "selected" && (
				<BaseEdge
					id={`${id}-trace-stripe`}
					path={path}
					interactionWidth={0}
					style={traceStroke(
						`color-mix(in oklch, ${color} 25%, var(--foreground) 75%)`,
						1.5,
						1,
					)}
				/>
			)}
			<BaseEdge
				id={`${id}-trace-flow`}
				path={path}
				interactionWidth={0}
				className="wire-trace-flow"
				style={{
					...traceStroke(
						`color-mix(in oklch, ${color} 50%, var(--foreground) 50%)`,
						3,
						1,
					),
					strokeDasharray: "0.1 31",
				}}
			/>
		</>
	);
}
