"use client";

import { memo, useMemo } from "react";
import type { IBoard } from "../../../../lib/schema/flow/board";
import { cn } from "../../../../lib/utils";
import { type IThumbnailKind, buildThumbnail } from "./templates-gallery-model";

/** Written out in full: Tailwind cannot see classes assembled at runtime. */
const BOX_CLASS: Record<IThumbnailKind, string> = {
	event: "fill-primary/10 stroke-primary/60",
	ai: "fill-card stroke-border",
	data: "fill-card stroke-border",
	logic: "fill-card stroke-border",
	io: "fill-card stroke-border",
	other: "fill-card stroke-border",
	layer: "fill-muted stroke-muted-foreground/40",
};

const DOT_CLASS: Record<IThumbnailKind, string> = {
	event: "fill-primary",
	ai: "fill-violet-500",
	data: "fill-teal-500",
	logic: "fill-slate-400",
	io: "fill-sky-500",
	other: "fill-muted-foreground/50",
	layer: "fill-muted-foreground/60",
};

export const TemplateThumbnail = memo(function TemplateThumbnail({
	board,
	className,
}: Readonly<{ board: IBoard; className?: string }>) {
	const thumbnail = useMemo(() => buildThumbnail(board), [board]);
	if (!thumbnail) return null;

	return (
		<svg
			aria-hidden="true"
			viewBox={thumbnail.viewBox}
			preserveAspectRatio="xMidYMid meet"
			className={cn("size-full", className)}
		>
			{thumbnail.comments.map((comment) => (
				<rect
					key={comment.id}
					x={comment.x}
					y={comment.y}
					width={comment.w}
					height={comment.h}
					rx={14}
					className="fill-foreground/5"
				/>
			))}
			<g fill="none">
				{thumbnail.wires.map((wire) => (
					<path
						key={wire.id}
						d={wire.d}
						strokeWidth={1.25}
						vectorEffect="non-scaling-stroke"
						className={
							wire.execution
								? "stroke-foreground/35"
								: "stroke-muted-foreground/30"
						}
					/>
				))}
			</g>
			{thumbnail.boxes.map((box) => (
				<g key={box.id}>
					<rect
						x={box.x}
						y={box.y}
						width={box.w}
						height={box.h}
						rx={12}
						strokeWidth={1}
						vectorEffect="non-scaling-stroke"
						strokeDasharray={box.kind === "layer" ? "5 3" : undefined}
						className={BOX_CLASS[box.kind]}
					/>
					<circle
						cx={box.x + 22}
						cy={box.y + 18}
						r={7}
						className={DOT_CLASS[box.kind]}
					/>
					<rect
						x={box.x + 38}
						y={box.y + 14}
						width={box.w - 58}
						height={8}
						rx={4}
						className="fill-muted-foreground/20"
					/>
				</g>
			))}
		</svg>
	);
});
