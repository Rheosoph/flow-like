"use client";
import { useMemo } from "react";
import { cn } from "../../lib/utils";

interface CourseBoardGlyphProps {
	/** Stable seed — the course id. The same course always draws the same board. */
	readonly seed: string;
	readonly className?: string;
	readonly accent?: boolean;
}

interface Layout {
	readonly nodes: ReadonlyArray<readonly [number, number, number]>;
	readonly wires: ReadonlyArray<readonly [number, number]>;
}

const NODE_H = 26;

/**
 * Courses without a banner get a board instead of a gradient: the same visual
 * language the learner is about to work in. Layouts are picked from the seed so
 * a course keeps its shape between renders and sessions.
 */
const LAYOUTS: ReadonlyArray<Layout> = [
	{
		nodes: [
			[16, 22, 68],
			[116, 22, 82],
			[230, 22, 54],
			[116, 80, 82],
		],
		wires: [
			[0, 1],
			[1, 2],
			[3, 1],
		],
	},
	{
		nodes: [
			[14, 18, 62],
			[100, 18, 68],
			[196, 18, 62],
			[100, 82, 68],
			[196, 82, 62],
		],
		wires: [
			[0, 1],
			[1, 2],
			[1, 3],
			[3, 4],
		],
	},
	{
		nodes: [
			[14, 8, 52],
			[14, 50, 52],
			[14, 92, 52],
			[128, 50, 62],
			[232, 50, 54],
		],
		wires: [
			[0, 3],
			[1, 3],
			[2, 3],
			[3, 4],
		],
	},
	{
		nodes: [
			[12, 18, 62],
			[98, 18, 62],
			[192, 50, 58],
			[98, 86, 58],
		],
		wires: [
			[0, 1],
			[1, 2],
			[2, 3],
			[3, 2],
		],
	},
	{
		nodes: [
			[14, 50, 52],
			[104, 20, 76],
			[104, 84, 58],
			[216, 50, 62],
		],
		wires: [
			[0, 1],
			[0, 2],
			[1, 3],
			[2, 3],
		],
	},
];

function hash(seed: string) {
	let h = 2166136261;
	for (let i = 0; i < seed.length; i++) {
		h ^= seed.charCodeAt(i);
		h = Math.imul(h, 16777619);
	}
	return Math.abs(h);
}

export function CourseBoardGlyph({
	seed,
	className,
	accent = false,
}: CourseBoardGlyphProps) {
	const layout = useMemo(() => LAYOUTS[hash(seed) % LAYOUTS.length], [seed]);
	const wire = accent ? "var(--primary)" : "var(--muted-foreground)";

	return (
		<div
			className={cn(
				"relative size-full bg-muted/40 [background-image:radial-gradient(var(--border)_1px,transparent_1px)] [background-size:15px_15px]",
				className,
			)}
		>
			<svg
				viewBox="0 0 300 132"
				preserveAspectRatio="xMidYMid meet"
				className="size-full"
				role="presentation"
			>
				<title>Board preview</title>
				{layout.wires.map(([from, to]) => {
					const a = layout.nodes[from];
					const b = layout.nodes[to];
					const x1 = a[0] + a[2];
					const y1 = a[1] + NODE_H / 2;
					const x2 = b[0];
					const y2 = b[1] + NODE_H / 2;
					const dx = Math.max(14, Math.abs(x2 - x1) / 2);
					return (
						<path
							key={`${from}-${to}`}
							d={`M${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`}
							fill="none"
							stroke={wire}
							strokeWidth={1.5}
							opacity={0.7}
						/>
					);
				})}
				{layout.nodes.map(([x, y, w]) => (
					<g key={`${x}-${y}`}>
						<rect
							x={x}
							y={y}
							width={w}
							height={NODE_H}
							rx={5}
							fill="var(--card)"
							stroke="var(--border)"
						/>
						<line
							x1={x}
							y1={y + 9}
							x2={x + w}
							y2={y + 9}
							stroke="var(--border)"
						/>
						<rect
							x={x + 5}
							y={y + 3.5}
							width={3}
							height={3}
							rx={1}
							fill={wire}
						/>
						<rect
							x={x + 6}
							y={y + 14}
							width={Math.max(12, w * 0.45)}
							height={3}
							rx={1.5}
							fill="var(--muted-foreground)"
							opacity={0.35}
						/>
						<circle cx={x} cy={y + NODE_H / 2} r={2.6} fill={wire} />
						<circle cx={x + w} cy={y + NODE_H / 2} r={2.6} fill={wire} />
					</g>
				))}
			</svg>
		</div>
	);
}
