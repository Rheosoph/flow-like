"use client";

import { memo, useMemo } from "react";
import type React from "react";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { cn } from "../../lib/utils";

export type LayoutAlgorithm = "layered-lr" | "force-directed" | "tree";

interface LayoutOption {
	id: LayoutAlgorithm;
	label: string;
	description: string;
}

const LAYOUT_OPTIONS: LayoutOption[] = [
	{
		id: "layered-lr",
		label: "Layered",
		description: "Arranges nodes in columns by dependency depth. Best for linear flows.",
	},
	{
		id: "force-directed",
		label: "Force-Directed",
		description: "Physics simulation pushes connected nodes apart. Good for clusters.",
	},
	{
		id: "tree",
		label: "Tree",
		description: "Hierarchical tree from root nodes. Best for branching flows.",
	},
];

// ─── Animated SVG Previews ──────────────────────────────────────────────

function AnimatedLayeredLR() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden>
			<style>{`
				@keyframes fadeSlide { 0% { opacity: 0; transform: translateX(-8px); } 100% { opacity: 1; transform: translateX(0); } }
				.lr-node { animation: fadeSlide 0.5s ease-out both; }
				.lr-edge { stroke-dasharray: 40; stroke-dashoffset: 40; animation: dash 1s ease-out 0.4s forwards; }
				@keyframes dash { to { stroke-dashoffset: 0; } }
			`}</style>
			{/* Column 1 */}
			<rect className="lr-node" style={{ animationDelay: "0s" }} x="5" y="30" width="22" height="14" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			{/* Column 2 */}
			<rect className="lr-node" style={{ animationDelay: "0.1s" }} x="42" y="12" width="22" height="14" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			<rect className="lr-node" style={{ animationDelay: "0.15s" }} x="42" y="48" width="22" height="14" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			{/* Column 3 */}
			<rect className="lr-node" style={{ animationDelay: "0.2s" }} x="79" y="12" width="22" height="14" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			<rect className="lr-node" style={{ animationDelay: "0.25s" }} x="79" y="48" width="22" height="14" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			{/* Edges */}
			<line className="lr-edge" x1="27" y1="37" x2="42" y2="19" stroke="currentColor" strokeWidth="1" opacity="0.5" />
			<line className="lr-edge" x1="27" y1="37" x2="42" y2="55" stroke="currentColor" strokeWidth="1" opacity="0.5" />
			<line className="lr-edge" style={{ animationDelay: "0.6s" }} x1="64" y1="19" x2="79" y2="19" stroke="currentColor" strokeWidth="1" opacity="0.5" />
			<line className="lr-edge" style={{ animationDelay: "0.6s" }} x1="64" y1="55" x2="79" y2="55" stroke="currentColor" strokeWidth="1" opacity="0.5" />
		</svg>
	);
}

function AnimatedForceDirected() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden>
			<style>{`
				@keyframes pulse { 0%, 100% { r: 6; } 50% { r: 7.5; } }
				@keyframes spread { 0% { opacity: 0; } 100% { opacity: 1; } }
				.fd-node { animation: spread 0.6s ease-out both; }
				.fd-edge { stroke-dasharray: 60; stroke-dashoffset: 60; animation: dashFD 1.2s ease-out 0.3s forwards; }
				.fd-pulse { animation: pulse 2s ease-in-out infinite; }
				@keyframes dashFD { to { stroke-dashoffset: 0; } }
			`}</style>
			{/* Cluster 1 */}
			<circle className="fd-node fd-pulse" style={{ animationDelay: "0s" }} cx="30" cy="28" r="6" fill="currentColor" opacity="0.25" stroke="currentColor" strokeWidth="1" />
			<circle className="fd-node fd-pulse" style={{ animationDelay: "0.1s" }} cx="18" cy="48" r="6" fill="currentColor" opacity="0.25" stroke="currentColor" strokeWidth="1" />
			<circle className="fd-node fd-pulse" style={{ animationDelay: "0.15s" }} cx="42" cy="50" r="6" fill="currentColor" opacity="0.25" stroke="currentColor" strokeWidth="1" />
			{/* Cluster 2 */}
			<circle className="fd-node fd-pulse" style={{ animationDelay: "0.2s" }} cx="80" cy="20" r="6" fill="currentColor" opacity="0.25" stroke="currentColor" strokeWidth="1" />
			<circle className="fd-node fd-pulse" style={{ animationDelay: "0.25s" }} cx="96" cy="38" r="6" fill="currentColor" opacity="0.25" stroke="currentColor" strokeWidth="1" />
			<circle className="fd-node fd-pulse" style={{ animationDelay: "0.3s" }} cx="75" cy="52" r="6" fill="currentColor" opacity="0.25" stroke="currentColor" strokeWidth="1" />
			{/* Intra-cluster edges */}
			<line className="fd-edge" x1="30" y1="28" x2="18" y2="48" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="fd-edge" x1="30" y1="28" x2="42" y2="50" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="fd-edge" x1="18" y1="48" x2="42" y2="50" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="fd-edge" x1="80" y1="20" x2="96" y2="38" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="fd-edge" x1="80" y1="20" x2="75" y2="52" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="fd-edge" x1="96" y1="38" x2="75" y2="52" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			{/* Bridge edge */}
			<line className="fd-edge" style={{ animationDelay: "0.8s" }} x1="42" y1="50" x2="75" y2="52" stroke="currentColor" strokeWidth="0.8" opacity="0.3" strokeDasharray="3 2" />
		</svg>
	);
}

function AnimatedTree() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden>
			<style>{`
				@keyframes treeDrop { 0% { opacity: 0; transform: translateY(-6px); } 100% { opacity: 1; transform: translateY(0); } }
				.tree-node { animation: treeDrop 0.4s ease-out both; }
				.tree-edge { stroke-dasharray: 40; stroke-dashoffset: 40; animation: dashTree 0.8s ease-out 0.3s forwards; }
				@keyframes dashTree { to { stroke-dashoffset: 0; } }
			`}</style>
			{/* Root */}
			<rect className="tree-node" style={{ animationDelay: "0s" }} x="49" y="3" width="22" height="12" rx="3" fill="currentColor" opacity="0.3" stroke="currentColor" strokeWidth="1" />
			{/* Level 1 */}
			<rect className="tree-node" style={{ animationDelay: "0.12s" }} x="15" y="30" width="20" height="11" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			<rect className="tree-node" style={{ animationDelay: "0.16s" }} x="50" y="30" width="20" height="11" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			<rect className="tree-node" style={{ animationDelay: "0.2s" }} x="85" y="30" width="20" height="11" rx="3" fill="currentColor" opacity="0.2" stroke="currentColor" strokeWidth="1" />
			{/* Level 2 */}
			<rect className="tree-node" style={{ animationDelay: "0.28s" }} x="3" y="56" width="18" height="10" rx="2.5" fill="currentColor" opacity="0.15" stroke="currentColor" strokeWidth="0.8" />
			<rect className="tree-node" style={{ animationDelay: "0.32s" }} x="27" y="56" width="18" height="10" rx="2.5" fill="currentColor" opacity="0.15" stroke="currentColor" strokeWidth="0.8" />
			<rect className="tree-node" style={{ animationDelay: "0.36s" }} x="51" y="56" width="18" height="10" rx="2.5" fill="currentColor" opacity="0.15" stroke="currentColor" strokeWidth="0.8" />
			<rect className="tree-node" style={{ animationDelay: "0.4s" }} x="80" y="56" width="18" height="10" rx="2.5" fill="currentColor" opacity="0.15" stroke="currentColor" strokeWidth="0.8" />
			<rect className="tree-node" style={{ animationDelay: "0.44s" }} x="102" y="56" width="15" height="10" rx="2.5" fill="currentColor" opacity="0.15" stroke="currentColor" strokeWidth="0.8" />
			{/* Edges root → level1 */}
			<line className="tree-edge" x1="60" y1="15" x2="25" y2="30" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="tree-edge" x1="60" y1="15" x2="60" y2="30" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			<line className="tree-edge" x1="60" y1="15" x2="95" y2="30" stroke="currentColor" strokeWidth="0.8" opacity="0.4" />
			{/* Edges level1 → level2 */}
			<line className="tree-edge" style={{ animationDelay: "0.5s" }} x1="25" y1="41" x2="12" y2="56" stroke="currentColor" strokeWidth="0.8" opacity="0.3" />
			<line className="tree-edge" style={{ animationDelay: "0.5s" }} x1="25" y1="41" x2="36" y2="56" stroke="currentColor" strokeWidth="0.8" opacity="0.3" />
			<line className="tree-edge" style={{ animationDelay: "0.5s" }} x1="60" y1="41" x2="60" y2="56" stroke="currentColor" strokeWidth="0.8" opacity="0.3" />
			<line className="tree-edge" style={{ animationDelay: "0.5s" }} x1="95" y1="41" x2="89" y2="56" stroke="currentColor" strokeWidth="0.8" opacity="0.3" />
			<line className="tree-edge" style={{ animationDelay: "0.5s" }} x1="95" y1="41" x2="109" y2="56" stroke="currentColor" strokeWidth="0.8" opacity="0.3" />
		</svg>
	);
}

const PREVIEW_MAP: Record<LayoutAlgorithm, () => React.JSX.Element> = {
	"layered-lr": AnimatedLayeredLR,
	"force-directed": AnimatedForceDirected,
	tree: AnimatedTree,
};

export interface AutoLayoutDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSelect: (algorithm: LayoutAlgorithm) => void;
}

export const AutoLayoutDialog = memo(function AutoLayoutDialog({
	open,
	onOpenChange,
	onSelect,
}: AutoLayoutDialogProps) {
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
						<span className="text-sm font-medium">{opt.label}</span>
						<span className="mt-0.5 text-xs text-muted-foreground leading-snug">
							{opt.description}
						</span>
					</button>
				);
			}),
		[onSelect, onOpenChange],
	);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-2xl" onDoubleClick={(e) => e.stopPropagation()}>
				<DialogHeader>
					<DialogTitle>Auto Layout</DialogTitle>
					<DialogDescription>
						Choose a layout algorithm to automatically arrange the nodes on the
						current layer.
					</DialogDescription>
				</DialogHeader>
				<div className="grid grid-cols-3 gap-3 mt-1">{cards}</div>
			</DialogContent>
		</Dialog>
	);
});
