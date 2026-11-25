"use client";

import { useTranslation } from "@flow-like/locales";
import { ImageIcon, Puzzle, Upload, Workflow } from "lucide-react";
import {
	type ReactNode,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { IShowcaseOutlineNode } from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import {
	FIXED_WIDTH_WIRES,
	type WireKind,
	type WireNode,
	isEmptyWire,
	outlineToWire,
} from "./model";

const line = "block h-[0.6em] rounded-full bg-muted-foreground/25";
const MAP_DOTS: [number, number][] = [
	[22, 30],
	[48, 58],
	[70, 24],
	[62, 72],
	[34, 66],
];
const BARS = [55, 80, 40, 95, 70, 60, 85];

type Context = "col" | "row";

function Leaf({
	kind,
	context,
}: Readonly<{ kind: WireKind; context: Context }>) {
	const inRow = context === "row";
	switch (kind) {
		case "heading":
			return (
				<span className="block h-[1.1em] w-2/5 min-w-[3em] rounded-full bg-foreground/35" />
			);
		case "text":
			return <span className={cn(line, inRow ? "w-full" : "w-3/5")} />;
		case "paragraph":
			return (
				<span className="flex flex-col gap-[0.45em]">
					<span className={cn(line, "w-full")} />
					<span className={cn(line, "w-11/12")} />
					<span className={cn(line, "w-2/3")} />
				</span>
			);
		case "stat":
			return (
				<span className="flex flex-col gap-[0.45em]">
					<span className={cn(line, "h-[0.5em] w-1/3")} />
					<span className="block h-[1.4em] w-1/2 rounded-[0.3em] bg-foreground/35" />
				</span>
			);
		case "image":
			return (
				<span
					className={cn(
						"grid place-items-center rounded-[0.4em] bg-linear-to-br from-muted-foreground/20 to-muted-foreground/5 text-muted-foreground/40",
						inRow ? "h-[2.6em] w-[4em]" : "h-[6em] w-full",
					)}
				>
					{inRow ? null : <ImageIcon className="h-[1.6em] w-[1.6em]" />}
				</span>
			);
		case "avatar":
			return (
				<span className="block h-[2.2em] w-[2.2em] rounded-full bg-muted-foreground/25" />
			);
		case "chart":
			return (
				<span className="flex h-[5em] items-end gap-[0.35em] border-b border-border">
					{BARS.map((h, i) => (
						<span
							// biome-ignore lint/suspicious/noArrayIndexKey: static decorative bars
							key={i}
							className="flex-1 rounded-t-[0.2em] bg-primary/60"
							style={{ height: `${h}%` }}
						/>
					))}
				</span>
			);
		case "table":
			return (
				<span className="flex flex-col gap-[0.55em]">
					{[0, 1, 2, 3].map((i) => (
						<span
							key={i}
							className="grid grid-cols-4 gap-[0.8em] border-b border-border/70 pb-[0.45em]"
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
		case "map":
			return (
				<span className="relative block h-[6em] overflow-hidden rounded-[0.4em] border border-border bg-muted/40">
					<span className="absolute inset-0 bg-[radial-gradient(60%_50%_at_30%_40%,color-mix(in_oklch,var(--primary)_16%,transparent),transparent_70%)]" />
					{MAP_DOTS.map(([x, y]) => (
						<span
							key={`${x}-${y}`}
							className="absolute h-[0.6em] w-[0.6em] rounded-full bg-primary/80"
							style={{ left: `${x}%`, top: `${y}%` }}
						/>
					))}
				</span>
			);
		case "button":
			return (
				<span className="block h-[1.5em] w-[4.5em] shrink-0 rounded-[0.4em] bg-primary/70" />
			);
		case "icon":
			return (
				<span className="block h-[1.3em] w-[1.3em] shrink-0 rounded-[0.3em] bg-muted-foreground/35" />
			);
		case "badge":
			return (
				<span className="block h-[1.2em] w-[3.6em] shrink-0 rounded-full bg-primary/20 ring-1 ring-primary/40" />
			);
		case "field":
		case "select":
			return (
				<span className="flex h-[2em] w-full items-center justify-between rounded-[0.35em] border border-border bg-background/60 px-[0.6em]">
					<span
						className={cn(line, "h-[0.5em] w-2/5 bg-muted-foreground/20")}
					/>
					{kind === "select" ? (
						<span className="block h-[0.5em] w-[0.8em] rounded-[0.15em] bg-muted-foreground/35" />
					) : null}
				</span>
			);
		case "check":
			return inRow ? (
				<span className="block h-[1.3em] w-[2.3em] shrink-0 rounded-full bg-muted-foreground/30" />
			) : (
				<span className="flex flex-col gap-[0.5em]">
					{[60, 45, 52].map((w) => (
						<span key={w} className="flex items-center gap-[0.5em]">
							<span className="block h-[0.9em] w-[0.9em] shrink-0 rounded-full border border-muted-foreground/50" />
							<span className={line} style={{ width: `${w}%` }} />
						</span>
					))}
				</span>
			);
		case "file":
			return (
				<span className="grid h-[4.5em] w-full place-items-center rounded-[0.4em] border border-dashed border-muted-foreground/40 text-muted-foreground/50">
					<Upload className="h-[1.5em] w-[1.5em]" />
				</span>
			);
		case "progress":
			return (
				<span className="block h-[0.6em] w-full overflow-hidden rounded-full bg-muted">
					<span className="block h-full w-3/5 rounded-full bg-primary/70" />
				</span>
			);
		case "divider":
			return <span className="block h-px w-full bg-border" />;
		case "spacer":
			return inRow ? null : <span className="block h-[0.3em]" />;
		case "list":
			return (
				<span className="flex flex-col gap-[0.55em]">
					{[80, 64, 72].map((w) => (
						<span key={w} className="flex items-center gap-[0.5em]">
							<span className="block h-[0.6em] w-[0.6em] shrink-0 rounded-[0.15em] bg-primary/60" />
							<span className={line} style={{ width: `${w}%` }} />
						</span>
					))}
				</span>
			);
		case "embed":
			return (
				<span className="grid h-[6em] w-full place-items-center rounded-[0.4em] border border-dashed border-muted-foreground/35 bg-muted/20 text-muted-foreground/50">
					<Puzzle className="h-[1.6em] w-[1.6em]" />
				</span>
			);
		default:
			return (
				<span className="block h-[1.2em] w-1/2 rounded-[0.3em] bg-muted-foreground/15" />
			);
	}
}

const isContainer = (n: WireNode) =>
	n.kind === "col" || n.kind === "row" || n.kind === "grid";

function Node({
	node,
	context,
}: Readonly<{ node: WireNode; context: Context }>) {
	if (!isContainer(node)) return <Leaf kind={node.kind} context={context} />;
	const cols = Math.min(3, Math.max(1, node.children.length));
	const childContext: Context = node.kind === "row" ? "row" : "col";
	return (
		<div
			className={cn(
				"min-w-0 gap-[0.7em]",
				node.kind === "row" && "flex items-center",
				node.kind === "col" && "flex flex-col",
				node.kind === "grid" && "grid",
				node.card &&
					"rounded-[0.5em] border border-border bg-card/70 p-[0.8em] shadow-xs",
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
					className={cn(
						"min-w-0",
						node.kind === "row" &&
							(FIXED_WIDTH_WIRES.has(c.kind) ? "shrink-0" : "flex-1"),
					)}
				>
					<Node node={c} context={childContext} />
				</div>
			))}
		</div>
	);
}

const THUMB_FONT = 8;
const THUMB_FONT_MIN = 5.5;
const THUMB_FONT_MAX = 11;
const THUMB_PADDING_EM = 1.2;
const THUMB_FILL = 0.9;

/** Scales the em-sized thumbnail so its content fills the fixed-ratio canvas. */
function useFitFont(enabled: boolean) {
	const boxRef = useRef<HTMLDivElement>(null);
	const innerRef = useRef<HTMLDivElement>(null);
	const fontRef = useRef(THUMB_FONT);
	const [font, setFont] = useState(THUMB_FONT);

	useLayoutEffect(() => {
		const box = boxRef.current;
		const inner = innerRef.current;
		if (!enabled || !box || !inner) return;
		const fit = () => {
			const current = fontRef.current;
			const used = inner.offsetHeight + 2 * THUMB_PADDING_EM * current;
			if (used <= 0 || box.clientHeight <= 0) return;
			const next = Math.min(
				THUMB_FONT_MAX,
				Math.max(
					THUMB_FONT_MIN,
					(current * box.clientHeight * THUMB_FILL) / used,
				),
			);
			if (Math.abs(next - current) < 0.1) return;
			fontRef.current = next;
			setFont(next);
		};
		fit();
		if (typeof ResizeObserver === "undefined") return;
		const ro = new ResizeObserver(fit);
		ro.observe(box);
		ro.observe(inner);
		return () => ro.disconnect();
	}, [enabled]);

	return { boxRef, innerRef, font };
}

/** A page's layout as boxes: component types only, no text, data or styling. */
export function Wireframe({
	outline,
	large = false,
	label,
	className,
}: Readonly<{
	outline: IShowcaseOutlineNode;
	large?: boolean;
	label: string;
	className?: string;
}>) {
	const { t } = useTranslation("store");
	const wire = useMemo(() => outlineToWire(outline), [outline]);
	const empty = useMemo(() => isEmptyWire(wire), [wire]);
	const { boxRef, innerRef, font } = useFitFont(!large && !empty);
	return (
		<div
			ref={boxRef}
			role="img"
			aria-label={label}
			className={cn(
				"relative w-full overflow-hidden bg-background/60",
				large
					? "min-h-72 p-[1.4em] text-[13px]"
					: "aspect-16/10 p-[1.2em] mask-[linear-gradient(to_bottom,black_86%,transparent)]",
				className,
			)}
			style={large ? undefined : { fontSize: `${font}px` }}
		>
			{empty ? (
				<div className="absolute inset-0 grid place-items-center p-4 text-center">
					<span className="flex flex-col items-center gap-1.5 text-xs text-muted-foreground">
						<Workflow className="h-4 w-4" aria-hidden="true" />
						{t(
							"showcasePageDynamic",
							"Its flow builds this page when it opens",
						)}
					</span>
				</div>
			) : (
				<div ref={innerRef}>
					<Node node={wire} context="col" />
				</div>
			)}
		</div>
	);
}

/** Window chrome around a page preview, with the page's route in the address bar. */
export function PageFrame({
	route,
	children,
	className,
}: Readonly<{
	route?: string | null;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<div
			className={cn(
				"overflow-hidden rounded-lg border border-border bg-card",
				className,
			)}
		>
			<div className="flex items-center gap-2 border-b border-border/70 bg-muted/30 px-2.5 py-1.5">
				<span className="flex gap-1" aria-hidden="true">
					{[0, 1, 2].map((i) => (
						<span
							key={i}
							className="h-1.5 w-1.5 rounded-full bg-muted-foreground/30"
						/>
					))}
				</span>
				{route ? (
					<span className="min-w-0 flex-1 truncate rounded-sm bg-background/60 px-2 py-0.5 text-center font-mono text-[10.5px] text-muted-foreground">
						{route}
					</span>
				) : null}
			</div>
			{children}
		</div>
	);
}
