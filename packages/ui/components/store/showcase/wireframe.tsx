"use client";

import { useMemo } from "react";
import type { IShowcaseOutlineNode } from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import { type WireNode, outlineToWire } from "./model";

const line = "block h-1.5 rounded-full bg-muted-foreground/25";
const MAP_DOTS: [number, number][] = [
	[22, 30],
	[48, 58],
	[70, 24],
	[62, 72],
	[34, 66],
];

function Leaf({ kind }: Readonly<{ kind: WireNode["kind"] }>) {
	switch (kind) {
		case "heading":
			return (
				<span className="block h-2.5 w-2/5 rounded-full bg-foreground/30" />
			);
		case "text":
			return (
				<span className="flex flex-col gap-1">
					<span className={cn(line, "w-full")} />
					<span className={cn(line, "w-3/4")} />
				</span>
			);
		case "media":
			return (
				<span className="block min-h-6 flex-1 rounded-md border border-border bg-muted/50" />
			);
		case "table":
			return (
				<span className="flex flex-1 flex-col gap-1.5">
					{[0, 1, 2, 3].map((i) => (
						<span
							key={i}
							className="grid grid-cols-4 gap-2 border-b border-border pb-1"
						>
							{[0, 1, 2, 3].map((j) => (
								<span
									key={j}
									className={cn(
										line,
										i === 0 && "bg-muted-foreground/45",
										(i * 7 + j * 11) % 3 === 0 ? "w-3/4" : "w-full",
									)}
								/>
							))}
						</span>
					))}
				</span>
			);
		case "chart":
			return (
				<span className="flex h-10 items-end gap-1 border-b border-border">
					{[55, 80, 40, 95, 70, 60].map((h, i) => (
						<span
							// biome-ignore lint/suspicious/noArrayIndexKey: static decorative bars
							key={i}
							className="flex-1 rounded-t-xs bg-primary/70"
							style={{ height: `${h}%` }}
						/>
					))}
				</span>
			);
		case "map":
			return (
				<span className="relative block min-h-8 flex-1 overflow-hidden rounded-md border border-border bg-card">
					<span className="absolute inset-0 bg-[radial-gradient(60%_50%_at_30%_40%,color-mix(in_oklch,var(--primary)_16%,transparent),transparent_70%)]" />
					{MAP_DOTS.map(([x, y]) => (
						<span
							key={`${x}-${y}`}
							className="absolute h-1.5 w-1.5 rounded-full bg-primary shadow-[0_0_0_3px_color-mix(in_oklch,var(--primary)_25%,transparent)]"
							style={{ left: `${x}%`, top: `${y}%` }}
						/>
					))}
				</span>
			);
		case "button":
			return <span className="block h-2.5 w-1/4 rounded-full bg-primary/80" />;
		case "field":
			return (
				<span className="block h-3 w-full rounded-sm border border-border bg-card" />
			);
		case "list":
			return (
				<span className="flex flex-col gap-1.5">
					{[80, 64, 72].map((w) => (
						<span key={w} className="flex items-center gap-1.5">
							<span className="block h-1.5 w-1.5 shrink-0 rounded-xs bg-primary/70" />
							<span className={line} style={{ width: `${w}%` }} />
						</span>
					))}
				</span>
			);
		default:
			return (
				<span className="block h-3 w-1/2 rounded-sm bg-muted-foreground/15" />
			);
	}
}

function Node({ node }: Readonly<{ node: WireNode }>) {
	if (
		node.children.length === 0 &&
		!["col", "row", "grid"].includes(node.kind)
	) {
		return <Leaf kind={node.kind} />;
	}
	const cols = Math.min(3, Math.max(1, node.children.length));
	return (
		<div
			className={cn(
				"gap-1.5",
				node.kind === "row" && "flex items-center",
				node.kind === "col" && "flex flex-col",
				node.kind === "grid" && "grid",
				node.card && "rounded-md border border-border bg-card p-1.5",
			)}
			style={
				node.kind === "grid"
					? { gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }
					: undefined
			}
		>
			{node.children.map((c, i) => (
				<div
					// biome-ignore lint/suspicious/noArrayIndexKey: outline nodes have no ids
					key={i}
					className={cn("min-w-0", node.kind === "row" && "flex-1")}
				>
					<Node node={c} />
				</div>
			))}
		</div>
	);
}

export function Wireframe({
	outline,
	large = false,
	label,
}: Readonly<{
	outline: IShowcaseOutlineNode;
	large?: boolean;
	label: string;
}>) {
	const wire = useMemo(() => outlineToWire(outline), [outline]);
	return (
		<div
			role="img"
			aria-label={label}
			className={cn(
				"w-full overflow-hidden rounded-lg border border-border bg-background",
				large ? "min-h-64 p-4" : "aspect-16/10 p-2.5",
			)}
		>
			<Node node={wire} />
		</div>
	);
}
