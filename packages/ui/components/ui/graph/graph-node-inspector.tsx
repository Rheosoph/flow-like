"use client";

import { Expand, X, Copy, Check, Eye, EyeOff, Filter } from "lucide-react";
import React, { useState, useCallback, useEffect, useRef, useMemo } from "react";
import { Button } from "../button";
import { ScrollArea } from "../scroll-area";
import { Badge } from "../badge";
import { Checkbox } from "../checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import type { SubgraphNode, SubgraphEdge } from "../../../state/backend-state/graph-state";
import { getGraphIcon } from "./icons";

export interface ConnectionInfo {
	label: string;
	direction: "outgoing" | "incoming";
	targetCaption: string;
	targetId: string;
}

export interface GraphNodeInspectorProps {
	node: SubgraphNode | null;
	connections?: ConnectionInfo[];
	onClose: () => void;
	onExpand?: () => void;
	onConnectionClick?: (nodeId: string) => void;
}

export type ValueKind = "string" | "number" | "boolean" | "date" | "vector" | "array" | "object" | "unknown";

export { inferValueKind, PropertyValue, FieldFilter, CopyButton };

function inferValueKind(value: unknown): { kind: ValueKind; dims?: number } {
	if (value === null || value === undefined) return { kind: "unknown" };
	if (typeof value === "boolean") return { kind: "boolean" };
	if (typeof value === "number") return { kind: "number" };
	if (Array.isArray(value)) {
		if (value.length > 0 && value.every((v) => typeof v === "number" || (typeof v === "string" && Number.isFinite(Number(v))))) {
			return { kind: "vector", dims: value.length };
		}
		return { kind: "array" };
	}
	if (typeof value === "object") return { kind: "object" };
	if (typeof value === "string") {
		if (/^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:?\d{2})?)?$/.test(value)) {
			return { kind: "date" };
		}
		return { kind: "string" };
	}
	return { kind: "unknown" };
}

function ensureNumericArray(v: unknown): number[] {
	if (Array.isArray(v)) return v.map(Number).filter((n) => Number.isFinite(n));
	return [];
}

const Sparkline: React.FC<{ data: number[]; width?: number; height?: number }> = ({ data, width = 120, height = 28 }) => {
	const ref = useRef<HTMLCanvasElement | null>(null);

	useEffect(() => {
		const canvas = ref.current;
		if (!canvas) return;

		const dpr = window.devicePixelRatio || 1;
		canvas.width = width * dpr;
		canvas.height = height * dpr;
		canvas.style.width = `${width}px`;
		canvas.style.height = `${height}px`;

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		ctx.scale(dpr, dpr);
		ctx.clearRect(0, 0, width, height);
		ctx.lineWidth = 1;
		ctx.beginPath();

		if (data.length === 0) return;

		const min = Math.min(...data);
		const max = Math.max(...data);
		const range = max - min || 1;

		for (let i = 0; i < data.length; i++) {
			const x = (i / Math.max(1, data.length - 1)) * (width - 2) + 1;
			const y = height - 1 - ((data[i] - min) / range) * (height - 2);
			if (i === 0) ctx.moveTo(x, y);
			else ctx.lineTo(x, y);
		}

		ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--primary");
		ctx.stroke();
	}, [data, width, height]);

	return <canvas ref={ref} aria-label="sparkline" />;
};

function formatDate(v: unknown): string {
	try {
		const d = new Date(v as string);
		if (Number.isNaN(d.getTime())) return String(v);
		return d.toLocaleString();
	} catch {
		return String(v);
	}
}

function CopyButton({ text }: { text: string }) {
	const [copied, setCopied] = useState(false);
	const handleCopy = useCallback(() => {
		navigator.clipboard.writeText(text);
		setCopied(true);
		setTimeout(() => setCopied(false), 1500);
	}, [text]);

	return (
		<button
			type="button"
			onClick={handleCopy}
			className="opacity-0 group-hover:opacity-100 transition-opacity p-0.5 rounded hover:bg-accent shrink-0"
			title="Copy value"
		>
			{copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3 text-muted-foreground" />}
		</button>
	);
}

function PropertyValue({ value, propKey }: { value: unknown; propKey: string }) {
	const { kind, dims } = inferValueKind(value);
	const display = typeof value === "object" ? JSON.stringify(value, null, 2) : String(value ?? "—");

	switch (kind) {
		case "boolean":
			return (
				<div className="group flex items-center justify-between">
					<div className="flex items-center gap-2">
						<Checkbox checked={value as boolean} disabled className="h-4 w-4" />
						<span className="text-sm">{value ? "true" : "false"}</span>
					</div>
					<CopyButton text={display} />
				</div>
			);

		case "vector": {
			const arr = ensureNumericArray(value);
			return (
				<div className="group space-y-1.5">
					<div className="flex items-center justify-between">
						<Badge variant="outline" className="text-[10px] px-1.5 py-0">
							{dims}d vector
						</Badge>
						<CopyButton text={display} />
					</div>
					<Sparkline data={arr.slice(0, 128)} />
				</div>
			);
		}

		case "number":
			return (
				<div className="group flex items-center justify-between">
					<span className="text-sm font-mono">{typeof value === "number" ? value.toLocaleString() : String(value)}</span>
					<CopyButton text={display} />
				</div>
			);

		case "date":
			return (
				<div className="group flex items-center justify-between">
					<span className="text-sm">{formatDate(value)}</span>
					<CopyButton text={String(value)} />
				</div>
			);

		case "array":
		case "object": {
			const json = JSON.stringify(value, null, 2);
			const isLong = json.length > 200;
			return (
				<div className="group relative">
					<pre className={`text-xs font-mono break-all whitespace-pre-wrap bg-muted/30 rounded p-1.5 ${isLong ? "max-h-32 overflow-y-auto" : ""}`}>
						{json}
					</pre>
					<div className="absolute top-1 right-1">
						<CopyButton text={json} />
					</div>
				</div>
			);
		}

		default: {
			const isLong = display.length > 120;
			return (
				<div className="group flex items-start justify-between gap-1">
					<p className={`text-sm break-all ${isLong ? "line-clamp-3" : ""}`}>{display}</p>
					<CopyButton text={display} />
				</div>
			);
		}
	}
}

function FieldFilter({
	allFields,
	hiddenFields,
	onToggle,
}: {
	allFields: string[];
	hiddenFields: Set<string>;
	onToggle: (field: string) => void;
}) {
	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7"
					title="Filter visible fields"
				>
					<Filter className="h-4 w-4" />
					{hiddenFields.size > 0 && (
						<span className="absolute -top-0.5 -right-0.5 h-3 w-3 rounded-full bg-primary text-[8px] text-primary-foreground flex items-center justify-center">
							{hiddenFields.size}
						</span>
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent className="w-56 p-2" align="end">
				<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2 px-1">
					Visible fields
				</p>
				<div className="space-y-1 max-h-60 overflow-y-auto">
					{allFields.map((field) => (
						<button
							key={field}
							type="button"
							className="w-full flex items-center gap-2 px-2 py-1 rounded text-sm hover:bg-accent transition-colors text-left"
							onClick={() => onToggle(field)}
						>
							{hiddenFields.has(field) ? (
								<EyeOff className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
							) : (
								<Eye className="h-3.5 w-3.5 text-foreground shrink-0" />
							)}
							<span className={hiddenFields.has(field) ? "text-muted-foreground line-through" : ""}>
								{field}
							</span>
						</button>
					))}
				</div>
			</PopoverContent>
		</Popover>
	);
}

export function GraphNodeInspector({ node, connections, onClose, onExpand, onConnectionClick }: GraphNodeInspectorProps) {
	const [hiddenFields, setHiddenFields] = useState<Set<string>>(new Set());

	if (!node) return null;

	const Icon = getGraphIcon(node.style?.icon ?? "database");
	const propEntries = node.props ? Object.entries(node.props).filter(([, v]) => v !== null && v !== undefined) : [];
	const allFields = propEntries.map(([k]) => k);
	const visibleEntries = propEntries.filter(([k]) => !hiddenFields.has(k));

	const handleToggleField = useCallback((field: string) => {
		setHiddenFields((prev) => {
			const next = new Set(prev);
			if (next.has(field)) next.delete(field);
			else next.add(field);
			return next;
		});
	}, []);

	return (
		<div className="w-80 shrink-0 bg-background border-l flex flex-col h-full min-h-0 overflow-hidden animate-in slide-in-from-right-5 duration-200">
			<div className="flex items-center justify-between p-4 border-b shrink-0">
				<div className="flex items-center gap-2 min-w-0">
					<div
						className="w-7 h-7 rounded-full flex items-center justify-center shrink-0 shadow-sm"
						style={{ backgroundColor: node.style?.color ?? "#64748b" }}
					>
						<Icon className="h-3.5 w-3.5 text-white" />
					</div>
					<div className="min-w-0">
						<h3 className="font-semibold text-sm truncate">{node.caption ?? node.id}</h3>
						<p className="text-xs text-muted-foreground">{node.label}</p>
					</div>
				</div>
				<div className="flex items-center gap-1 shrink-0">
					{allFields.length > 0 && (
						<FieldFilter
							allFields={allFields}
							hiddenFields={hiddenFields}
							onToggle={handleToggleField}
						/>
					)}
					{onExpand && (
						<Button variant="ghost" size="icon" className="h-7 w-7" onClick={onExpand} title="Expand neighbors (Shift+Click)">
							<Expand className="h-4 w-4" />
						</Button>
					)}
					<Button variant="ghost" size="icon" className="h-7 w-7" onClick={onClose}>
						<X className="h-4 w-4" />
					</Button>
				</div>
			</div>
			<ScrollArea className="flex-1 min-h-0">
				<div className="space-y-4 p-4">
					<div>
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-1">
							ID
						</p>
						<p className="text-xs font-mono break-all text-muted-foreground">{node.id}</p>
					</div>
					{visibleEntries.length > 0 && (
						<div>
							<div className="flex items-center justify-between mb-2">
								<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									Properties
								</p>
								{hiddenFields.size > 0 && (
									<span className="text-[10px] text-muted-foreground">
										{hiddenFields.size} hidden
									</span>
								)}
							</div>
							<div className="space-y-2">
								{visibleEntries.map(([key, value]) => (
									<div key={key} className="rounded-md bg-muted/50 px-3 py-2">
										<div className="flex items-center justify-between mb-0.5">
											<p className="text-[10px] font-medium text-muted-foreground">{key}</p>
											<span className="text-[9px] text-muted-foreground/60">{inferValueKind(value).kind}</span>
										</div>
										<PropertyValue value={value} propKey={key} />
									</div>
								))}
							</div>
						</div>
					)}
					{propEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">No properties available</p>
					)}
					{propEntries.length > 0 && visibleEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">All fields hidden — use the filter to show them</p>
					)}

					{/* Connections section */}
					{connections && connections.length > 0 && (
						<div>
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2">
								Connections ({connections.length})
							</p>
							<div className="space-y-1">
								{connections.map((conn, i) => (
								<button
									type="button"
									key={`${conn.direction}-${conn.label}-${conn.targetId}-${i}`}
									className="w-full rounded-md bg-muted/50 px-3 py-1.5 flex items-center gap-2 text-xs hover:bg-accent transition-colors text-left cursor-pointer"
									onClick={() => onConnectionClick?.(conn.targetId)}
								>
									<span className={`shrink-0 text-[10px] font-medium ${conn.direction === "outgoing" ? "text-blue-500" : "text-amber-500"}`}>
										{conn.direction === "outgoing" ? "→" : "←"}
									</span>
									<span className="font-medium text-muted-foreground shrink-0">{conn.label}</span>
									<span className="truncate">{conn.targetCaption}</span>
								</button>
								))}
							</div>
						</div>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
