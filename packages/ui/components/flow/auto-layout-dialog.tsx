"use client";

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
	id: LayoutStyle;
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
		id: "balanced",
		label: "Balanced",
		description: "Even spacing between execution chains and data dependencies.",
	},
	{
		id: "expanded",
		label: "Expanded",
		description:
			"Extra room between event groups and branches for readability.",
	},
];

// ─── Animated SVG Previews ──────────────────────────────────────────────

function AnimatedCompact() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden>
			<style>{`
				@keyframes fadeSlide { 0% { opacity: 0; transform: translateX(-8px); } 100% { opacity: 1; transform: translateX(0); } }
				.cp-node { animation: fadeSlide 0.5s ease-out both; }
				.cp-edge { stroke-dasharray: 40; stroke-dashoffset: 40; animation: cpDash 1s ease-out 0.4s forwards; }
				@keyframes cpDash { to { stroke-dashoffset: 0; } }
			`}</style>
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

function AnimatedBalanced() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden>
			<style>{`
				@keyframes balSlide { 0% { opacity: 0; transform: translateX(-6px); } 100% { opacity: 1; transform: translateX(0); } }
				.bal-node { animation: balSlide 0.5s ease-out both; }
				.bal-edge { stroke-dasharray: 40; stroke-dashoffset: 40; animation: balDash 1s ease-out 0.4s forwards; }
				@keyframes balDash { to { stroke-dashoffset: 0; } }
			`}</style>
			<rect
				className="bal-node"
				style={{ animationDelay: "0s" }}
				x="3"
				y="15"
				width="20"
				height="13"
				rx="3"
				fill="hsl(30 80% 55%)"
				opacity="0.6"
				stroke="hsl(30 80% 55%)"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.1s" }}
				x="32"
				y="15"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.2s" }}
				x="61"
				y="6"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.2s" }}
				x="61"
				y="25"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.3s" }}
				x="90"
				y="15"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.12s" }}
				x="16"
				y="1"
				width="16"
				height="10"
				rx="2"
				fill="currentColor"
				opacity="0.12"
				stroke="currentColor"
				strokeWidth="0.6"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.22s" }}
				x="45"
				y="1"
				width="16"
				height="10"
				rx="2"
				fill="currentColor"
				opacity="0.12"
				stroke="currentColor"
				strokeWidth="0.6"
			/>
			<line
				className="bal-edge"
				x1="23"
				y1="21"
				x2="32"
				y2="21"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="bal-edge"
				x1="52"
				y1="21"
				x2="61"
				y2="12"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="bal-edge"
				x1="52"
				y1="21"
				x2="61"
				y2="31"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="bal-edge"
				x1="81"
				y1="12"
				x2="90"
				y2="21"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="bal-edge"
				x1="81"
				y1="31"
				x2="90"
				y2="21"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="bal-edge"
				x1="32"
				y1="6"
				x2="42"
				y2="15"
				stroke="currentColor"
				strokeWidth="0.5"
				opacity="0.3"
				strokeDasharray="2 1.5"
			/>
			<line
				className="bal-edge"
				x1="61"
				y1="6"
				x2="61"
				y2="6"
				stroke="currentColor"
				strokeWidth="0.5"
				opacity="0.3"
				strokeDasharray="2 1.5"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0s" }}
				x="3"
				y="55"
				width="20"
				height="13"
				rx="3"
				fill="hsl(30 80% 55%)"
				opacity="0.6"
				stroke="hsl(30 80% 55%)"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.1s" }}
				x="32"
				y="55"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="bal-node"
				style={{ animationDelay: "0.2s" }}
				x="61"
				y="55"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<line
				className="bal-edge"
				x1="23"
				y1="61"
				x2="32"
				y2="61"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="bal-edge"
				x1="52"
				y1="61"
				x2="61"
				y2="61"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
		</svg>
	);
}

function AnimatedExpanded() {
	return (
		<svg viewBox="0 0 120 80" className="w-full h-full" aria-hidden>
			<style>{`
				@keyframes expSlide { 0% { opacity: 0; transform: translateX(-6px); } 100% { opacity: 1; transform: translateX(0); } }
				.exp-node { animation: expSlide 0.6s ease-out both; }
				.exp-edge { stroke-dasharray: 40; stroke-dashoffset: 40; animation: expDash 1s ease-out 0.4s forwards; }
				@keyframes expDash { to { stroke-dashoffset: 0; } }
			`}</style>
			<rect
				className="exp-node"
				style={{ animationDelay: "0s" }}
				x="2"
				y="10"
				width="20"
				height="13"
				rx="3"
				fill="hsl(30 80% 55%)"
				opacity="0.6"
				stroke="hsl(30 80% 55%)"
				strokeWidth="0.8"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0.12s" }}
				x="36"
				y="10"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0.24s" }}
				x="70"
				y="3"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0.24s" }}
				x="70"
				y="20"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0.36s" }}
				x="100"
				y="10"
				width="17"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<line
				className="exp-edge"
				x1="22"
				y1="16"
				x2="36"
				y2="16"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="exp-edge"
				x1="56"
				y1="16"
				x2="70"
				y2="9"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="exp-edge"
				x1="56"
				y1="16"
				x2="70"
				y2="26"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="exp-edge"
				x1="90"
				y1="9"
				x2="100"
				y2="16"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="exp-edge"
				x1="90"
				y1="26"
				x2="100"
				y2="16"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0s" }}
				x="2"
				y="58"
				width="20"
				height="13"
				rx="3"
				fill="hsl(30 80% 55%)"
				opacity="0.6"
				stroke="hsl(30 80% 55%)"
				strokeWidth="0.8"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0.12s" }}
				x="36"
				y="58"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<rect
				className="exp-node"
				style={{ animationDelay: "0.24s" }}
				x="70"
				y="58"
				width="20"
				height="13"
				rx="3"
				fill="currentColor"
				opacity="0.2"
				stroke="currentColor"
				strokeWidth="0.8"
			/>
			<line
				className="exp-edge"
				x1="22"
				y1="64"
				x2="36"
				y2="64"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
			<line
				className="exp-edge"
				x1="56"
				y1="64"
				x2="70"
				y2="64"
				stroke="currentColor"
				strokeWidth="0.8"
				opacity="0.5"
			/>
		</svg>
	);
}

const PREVIEW_MAP: Record<LayoutStyle, () => React.JSX.Element> = {
	compact: AnimatedCompact,
	balanced: AnimatedBalanced,
	expanded: AnimatedExpanded,
};

export interface AutoLayoutDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSelect: (style: LayoutStyle) => void;
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
			<DialogContent
				className="sm:max-w-2xl"
				onDoubleClick={(e) => e.stopPropagation()}
			>
				<DialogHeader>
					<DialogTitle>Auto Layout</DialogTitle>
					<DialogDescription>
						Arrange nodes left-to-right following execution flow. Events are
						grouped separately with data nodes placed beside their consumers.
					</DialogDescription>
				</DialogHeader>
				<div className="grid grid-cols-3 gap-3 mt-1">{cards}</div>
			</DialogContent>
		</Dialog>
	);
});
