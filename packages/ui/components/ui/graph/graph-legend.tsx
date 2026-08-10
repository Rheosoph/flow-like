"use client";

import { Eye, EyeOff } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import type { LabelStyle } from "../../../state/backend-state/graph-state";
import { GRAPH_ICONS, type IconKey, getGraphIcon } from "./icons";

export interface LegendEntry {
	label: string;
	style: LabelStyle;
	count?: number;
	/** Whole-population size, when it is known exactly for this label. */
	total?: number;
	type: "node" | "edge";
}

export interface GraphLegendProps {
	entries: LegendEntry[];
	onToggleVisibility?: (label: string, visible: boolean) => void;
	onStyleChange?: (
		label: string,
		type: "node" | "edge",
		style: LabelStyle,
	) => void;
}

function InlineColorPicker({
	color,
	onChange,
}: { color: string; onChange: (color: string) => void }) {
	const inputRef = useRef<HTMLInputElement>(null);
	return (
		<button
			type="button"
			className="relative w-5 h-5 rounded shrink-0 border border-border/50 cursor-pointer"
			style={{ backgroundColor: color }}
			onClick={(e) => {
				e.stopPropagation();
				inputRef.current?.click();
			}}
			title="Change color"
		>
			<input
				ref={inputRef}
				type="color"
				value={color}
				onChange={(e) => onChange(e.target.value)}
				onClick={(e) => e.stopPropagation()}
				className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
			/>
		</button>
	);
}

function InlineIconPicker({
	icon,
	color,
	onChange,
}: { icon: string; color: string; onChange: (icon: string) => void }) {
	const [open, setOpen] = useState(false);
	const Icon = getGraphIcon(icon);
	const iconKeys = Object.keys(GRAPH_ICONS) as IconKey[];

	return (
		<div className="relative">
			<button
				type="button"
				className="w-5 h-5 rounded-full shrink-0 flex items-center justify-center ring-2 ring-transparent hover:ring-accent transition-all"
				style={{ backgroundColor: color }}
				onClick={(e) => {
					e.stopPropagation();
					setOpen(!open);
				}}
				title="Change icon"
			>
				<Icon className="h-2.5 w-2.5 text-white" />
			</button>
			{open && (
				<div
					className="absolute bottom-full left-0 mb-1 bg-popover border rounded-lg shadow-lg p-2 z-50 grid grid-cols-6 gap-1 w-[180px]"
					onClick={(e) => e.stopPropagation()}
					onKeyDown={() => {}}
				>
					{iconKeys.map((key) => {
						const ItemIcon = GRAPH_ICONS[key];
						const isActive = key === icon;
						return (
							<button
								type="button"
								key={key}
								className={`w-6 h-6 rounded flex items-center justify-center transition-colors ${isActive ? "bg-primary text-primary-foreground" : "hover:bg-accent"}`}
								onClick={() => {
									onChange(key);
									setOpen(false);
								}}
								title={key}
							>
								<ItemIcon className="h-3.5 w-3.5" />
							</button>
						);
					})}
				</div>
			)}
		</div>
	);
}

export function GraphLegend({
	entries,
	onToggleVisibility,
	onStyleChange,
}: GraphLegendProps) {
	const [hidden, setHidden] = useState<Set<string>>(new Set());

	const toggle = (label: string) => {
		setHidden((prev) => {
			const next = new Set(prev);
			if (next.has(label)) {
				next.delete(label);
				onToggleVisibility?.(label, true);
			} else {
				next.add(label);
				onToggleVisibility?.(label, false);
			}
			return next;
		});
	};

	const handleColorChange = useCallback(
		(entry: LegendEntry, color: string) => {
			onStyleChange?.(entry.label, entry.type, { ...entry.style, color });
		},
		[onStyleChange],
	);

	const handleIconChange = useCallback(
		(entry: LegendEntry, icon: string) => {
			onStyleChange?.(entry.label, entry.type, { ...entry.style, icon });
		},
		[onStyleChange],
	);

	const nodeEntries = entries.filter((e) => e.type === "node");
	const edgeEntries = entries.filter((e) => e.type === "edge");

	return (
		<div className="bg-background/80 backdrop-blur-sm rounded-lg border p-3 shadow-sm space-y-3 max-w-[200px]">
			{nodeEntries.length > 0 && (
				<div className="space-y-1.5">
					<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
						Nodes
					</p>
					{nodeEntries.map((entry) => {
						const isHidden = hidden.has(entry.label);
						return (
							<div
								key={`node-${entry.label}`}
								className="flex items-center gap-2 text-xs"
							>
								{onStyleChange ? (
									<div className="flex items-center gap-1 shrink-0">
										<InlineIconPicker
											icon={entry.style.icon}
											color={entry.style.color}
											onChange={(icon) => handleIconChange(entry, icon)}
										/>
										<InlineColorPicker
											color={entry.style.color}
											onChange={(color) => handleColorChange(entry, color)}
										/>
									</div>
								) : (
									<div
										className="w-3 h-3 rounded-full shrink-0 flex items-center justify-center"
										style={{ backgroundColor: entry.style.color }}
									>
										{(() => {
											const Icon = getGraphIcon(entry.style.icon);
											return <Icon className="h-2 w-2 text-white" />;
										})()}
									</div>
								)}
								<span
									className={
										isHidden ? "text-muted-foreground line-through" : ""
									}
								>
									{entry.label}
								</span>
								{entry.count !== undefined && (
									<span className="ml-auto text-muted-foreground tabular-nums">
										{entry.count.toLocaleString()}
										{entry.total !== undefined && (
											<span className="opacity-60">
												{" / "}
												{entry.total.toLocaleString()}
											</span>
										)}
									</span>
								)}
								<button
									type="button"
									onClick={() => toggle(entry.label)}
									className="shrink-0 hover:text-foreground transition-colors"
									title={isHidden ? "Show" : "Hide"}
								>
									{isHidden ? (
										<EyeOff className="h-3 w-3 text-muted-foreground" />
									) : (
										<Eye className="h-3 w-3 text-muted-foreground" />
									)}
								</button>
							</div>
						);
					})}
				</div>
			)}
			{edgeEntries.length > 0 && (
				<div className="space-y-1.5">
					<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
						Edges
					</p>
					{edgeEntries.map((entry) => {
						const isHidden = hidden.has(entry.label);
						return (
							<div
								key={`edge-${entry.label}`}
								className="flex items-center gap-2 text-xs"
							>
								{onStyleChange ? (
									<InlineColorPicker
										color={entry.style.color}
										onChange={(color) => handleColorChange(entry, color)}
									/>
								) : null}
								<div
									className="w-3 h-0.5 shrink-0"
									style={{ backgroundColor: entry.style.color }}
								/>
								<span
									className={
										isHidden ? "text-muted-foreground line-through" : ""
									}
								>
									{entry.label}
								</span>
								{entry.count !== undefined && (
									<span className="ml-auto text-muted-foreground tabular-nums">
										{entry.count.toLocaleString()}
										{entry.total !== undefined && (
											<span className="opacity-60">
												{" / "}
												{entry.total.toLocaleString()}
											</span>
										)}
									</span>
								)}
								<button
									type="button"
									onClick={() => toggle(entry.label)}
									className="shrink-0 hover:text-foreground transition-colors"
									title={isHidden ? "Show" : "Hide"}
								>
									{isHidden ? (
										<EyeOff className="h-3 w-3 text-muted-foreground" />
									) : (
										<Eye className="h-3 w-3 text-muted-foreground" />
									)}
								</button>
							</div>
						);
					})}
				</div>
			)}
		</div>
	);
}
