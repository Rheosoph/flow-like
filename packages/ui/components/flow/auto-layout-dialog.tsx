"use client";

import { useTranslation } from "@flow-like/locales";
import { memo, useMemo } from "react";
import type React from "react";
import type { LayoutStyle } from "../../lib/flow-auto-layout";
import { cn } from "../../lib/utils";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";

export type { LayoutStyle };

interface LayoutOption {
	id: Extract<LayoutStyle, "compact" | "routed">;
	label: string;
	description: string;
}

const LAYOUT_OPTIONS: LayoutOption[] = [
	{
		id: "compact",
		label: "Compact",
		description: "Tight spacing. Pure nodes packed close to their consumers.",
	},
	{
		id: "routed",
		label: "Routed",
		description:
			"Compact spacing with reroutes to avoid nodes and reduce crossings.",
	},
];

// ─── Animated SVG Previews ──────────────────────────────────────────────

function AnimatedCompact() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden="true">
			<style>
				{
					"@keyframes fadeSlide { 0% { opacity: 0; transform: translateX(-8px); } 100% { opacity: 1; transform: translateX(0); } } .cp-node { animation: fadeSlide 0.5s ease-out both; } .cp-edge { stroke-dasharray: 40; stroke-dashoffset: 40; animation: cpDash 1s ease-out 0.4s forwards; } @keyframes cpDash { to { stroke-dashoffset: 0; } }"
				}
			</style>
			<rect
				className="cp-node"
				style={{ animationDelay: "0s" }}
				x="3"
				y="18"
				width="18"
				height="12"
				rx="3"
				fill="hsl(30 80% 55%)"
				opacity="0.6"
				stroke="hsl(30 80% 55%)"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.08s" }}
				x="28"
				y="18"
				width="18"
				height="12"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.16s" }}
				x="53"
				y="10"
				width="18"
				height="12"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.16s" }}
				x="53"
				y="26"
				width="18"
				height="12"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.24s" }}
				x="78"
				y="18"
				width="18"
				height="12"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.1s" }}
				x="15"
				y="5"
				width="14"
				height="9"
				rx="2"
				fill="currentColor"
				opacity="0.12"
				stroke="currentColor"
				strokeWidth="0.6"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.18s" }}
				x="40"
				y="2"
				width="14"
				height="9"
				rx="2"
				fill="currentColor"
				opacity="0.12"
				stroke="currentColor"
				strokeWidth="0.6"
			/>
			<line
				className="cp-edge"
				x1="21"
				y1="24"
				x2="28"
				y2="24"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="cp-edge"
				x1="46"
				y1="24"
				x2="53"
				y2="16"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="cp-edge"
				x1="46"
				y1="24"
				x2="53"
				y2="32"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="cp-edge"
				x1="71"
				y1="16"
				x2="78"
				y2="24"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="cp-edge"
				x1="71"
				y1="32"
				x2="78"
				y2="24"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="cp-edge"
				x1="29"
				y1="9"
				x2="37"
				y2="18"
				stroke="currentColor"
				strokeWidth="0.5"
				opacity="0.3"
				strokeDasharray="2 1.5"
			/>
			<line
				className="cp-edge"
				x1="54"
				y1="9"
				x2="53"
				y2="10"
				stroke="currentColor"
				strokeWidth="0.5"
				opacity="0.3"
				strokeDasharray="2 1.5"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0s" }}
				x="3"
				y="52"
				width="18"
				height="12"
				rx="3"
				fill="hsl(30 80% 55%)"
				opacity="0.6"
				stroke="hsl(30 80% 55%)"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.08s" }}
				x="28"
				y="52"
				width="18"
				height="12"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="cp-node"
				style={{ animationDelay: "0.16s" }}
				x="53"
				y="52"
				width="18"
				height="12"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<line
				className="cp-edge"
				x1="21"
				y1="58"
				x2="28"
				y2="58"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="cp-edge"
				x1="46"
				y1="58"
				x2="53"
				y2="58"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
		</svg>
	);
}

function AnimatedRouted() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden="true">
			<style>
				{
					"@keyframes routedAppear { from { opacity: 0; } to { opacity: 1; } } @keyframes routedDraw { to { stroke-dashoffset: 0; } } .routed-node { animation: routedAppear 0.5s ease-out both; } .routed-wire { stroke-dasharray: 1; stroke-dashoffset: 1; animation: routedDraw 0.8s ease-out 0.3s forwards; } .routed-dot { animation: routedAppear 0.3s ease-out 0.65s both; } @media (prefers-reduced-motion: reduce) { .routed-node, .routed-wire, .routed-dot { animation: none; stroke-dashoffset: 0; } }"
				}
			</style>
			<g className="routed-node" strokeWidth="0.8">
				<rect
					x="3"
					y="19"
					width="25"
					height="29"
					rx="3"
					fill="hsl(30 80% 55% / 0.35)"
					stroke="hsl(30 80% 55%)"
				/>
				<rect
					x="48"
					y="19"
					width="25"
					height="29"
					rx="3"
					fill="currentColor"
					fillOpacity="0.15"
					stroke="currentColor"
					strokeOpacity="0.5"
				/>
				<rect
					x="93"
					y="19"
					width="24"
					height="29"
					rx="3"
					fill="currentColor"
					fillOpacity="0.15"
					stroke="currentColor"
					strokeOpacity="0.5"
				/>
				<path
					d="M28 27 H48 M73 27 H93"
					fill="none"
					stroke="currentColor"
					strokeOpacity="0.6"
				/>
			</g>
			<path
				className="routed-wire"
				d="M28 38 C34 38 32 61 39 61 H82 C89 61 87 38 93 38"
				pathLength="1"
				fill="none"
				stroke="hsl(275 85% 62%)"
				strokeWidth="1.2"
			/>
			<g className="routed-dot" fill="hsl(275 85% 62%)">
				<circle cx="28" cy="38" r="1.7" />
				<circle cx="39" cy="61" r="2.5" />
				<circle cx="82" cy="61" r="2.5" />
				<circle cx="93" cy="38" r="1.7" />
			</g>
		</svg>
	);
}

const PREVIEW_MAP: Record<LayoutOption["id"], () => React.JSX.Element> = {
	compact: AnimatedCompact,
	routed: AnimatedRouted,
};

export interface AutoLayoutDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSelect: (style: LayoutStyle) => void;
	/** Number of currently selected nodes; >1 scopes the layout to them. */
	selectionCount?: number;
}

export const AutoLayoutDialog = memo(function AutoLayoutDialog({
	open,
	onOpenChange,
	onSelect,
	selectionCount = 0,
}: AutoLayoutDialogProps) {
	const { t } = useTranslation("flow");
	const scoped = selectionCount > 1;
	const cards = useMemo(
		() =>
			LAYOUT_OPTIONS.map((opt) => {
				const Preview = PREVIEW_MAP[opt.id];
				return (
					<button
						key={opt.id}
						type="button"
						className={cn(
							"group relative flex flex-col rounded-xl border bg-card p-3 text-left transition-all",
							"hover:border-primary/50 hover:shadow-md hover:shadow-primary/5",
							"focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
							"cursor-pointer",
						)}
						onClick={() => {
							onSelect(opt.id);
							onOpenChange(false);
						}}
					>
						<div className="mb-2 aspect-3/2 w-full overflow-hidden rounded-lg bg-muted/50 text-muted-foreground p-1">
							<Preview />
						</div>
						<span className="text-sm font-medium">
							{t(`${opt.id}Layout`, opt.label)}
						</span>
						<span className="mt-0.5 text-xs text-muted-foreground leading-snug">
							{t(`${opt.id}LayoutDescription`, opt.description)}
						</span>
					</button>
				);
			}),
		[onSelect, onOpenChange, t],
	);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				className="sm:max-w-lg"
				onDoubleClick={(e) => e.stopPropagation()}
			>
				<DialogHeader>
					<DialogTitle>{t("autoLayout", "Auto Layout")}</DialogTitle>
					<DialogDescription>
						{scoped
							? t(
									"arrangesTheSelectioncountSelectedNodesLefttorightAlongTheirExecutionFlowTheRestOfTheBoardStaysWhereItIs",
									"Arranges the {{selectionCount}} selected nodes left-to-right along their execution flow. The rest of the board stays where it is.",
									{ selectionCount },
								)
							: t(
									"arrangesEveryNodeInThisLayerLefttorightAlongItsExecutionFlowWithDataNodesBandedBeneathTheNodeThatConsumesThem",
									"Arranges every node in this layer left-to-right along its execution flow, with data nodes banded beneath the node that consumes them.",
								)}
					</DialogDescription>
				</DialogHeader>
				<div className="grid grid-cols-2 gap-3 mt-1">{cards}</div>
				<p className="text-xs text-muted-foreground">
					{scoped
						? t(
								"pressEscapeAndClearTheSelectionToLayOutTheWholeLayer",
								"Press Escape and clear the selection to lay out the whole layer.",
							)
						: t(
								"selectTwoOrMoreNodesFirstToLayOutOnlyThatPartOfTheBoard",
								"Select two or more nodes first to lay out only that part of the board.",
							)}{" "}
					{t("undoWithZ", "Undo with ⌘Z.")}
				</p>
			</DialogContent>
		</Dialog>
	);
});
