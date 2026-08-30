"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRight, X } from "lucide-react";
import { useCallback, useState } from "react";
import type { SubgraphEdge } from "../../../state/backend-state/graph-state";
import { Button } from "../button";
import { ScrollArea } from "../scroll-area";
import {
	FieldFilter,
	PropertyValue,
	inferValueKind,
} from "./graph-node-inspector";
import { getGraphIcon } from "./icons";

export interface GraphEdgeInspectorProps {
	edge: SubgraphEdge | null;
	sourceCaption?: string;
	targetCaption?: string;
	onClose: () => void;
}

export function GraphEdgeInspector({
	edge,
	sourceCaption,
	targetCaption,
	onClose,
}: GraphEdgeInspectorProps) {
	const { t } = useTranslation("common");
	const [hiddenFields, setHiddenFields] = useState<Set<string>>(new Set());

	if (!edge) return null;

	const Icon = getGraphIcon(edge.style?.icon ?? "link");
	const propEntries = edge.props
		? Object.entries(edge.props).filter(
				([, v]) => v !== null && v !== undefined,
			)
		: [];
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
						style={{ backgroundColor: edge.style?.color ?? "#94a3b8" }}
					>
						<Icon className="h-3.5 w-3.5 text-white" />
					</div>
					<div className="min-w-0">
						<h3 className="font-semibold text-sm truncate">{edge.label}</h3>
						<p className="text-xs text-muted-foreground">{t("edge", "Edge")}</p>
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
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						onClick={onClose}
					>
						<X className="h-4 w-4" />
					</Button>
				</div>
			</div>
			<ScrollArea className="flex-1 min-h-0">
				<div className="space-y-4 p-4">
					{/* Source → Target */}
					<div className="rounded-md bg-muted/50 px-3 py-2 space-y-1">
						<div className="flex items-center gap-2 text-sm">
							<span className="truncate font-medium">
								{sourceCaption ?? edge.source}
							</span>
							<ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
							<span className="truncate font-medium">
								{targetCaption ?? edge.target}
							</span>
						</div>
						<div className="flex items-center gap-2 text-[10px] font-mono text-muted-foreground">
							<span className="truncate">{edge.source}</span>
							<ArrowRight className="h-2.5 w-2.5 shrink-0" />
							<span className="truncate">{edge.target}</span>
						</div>
					</div>

					<div>
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-1">
							ID
						</p>
						<p className="text-xs font-mono break-all text-muted-foreground">
							{edge.id}
						</p>
					</div>

					{visibleEntries.length > 0 && (
						<div>
							<div className="flex items-center justify-between mb-2">
								<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									{t("properties", "Properties")}
								</p>
								{hiddenFields.size > 0 && (
									<span className="text-[10px] text-muted-foreground">
										{t("sizeHidden", "{{size}} hidden", {
											size: hiddenFields.size,
										})}
									</span>
								)}
							</div>
							<div className="space-y-2">
								{visibleEntries.map(([key, value]) => (
									<div key={key} className="rounded-md bg-muted/50 px-3 py-2">
										<div className="flex items-center justify-between mb-0.5">
											<p className="text-[10px] font-medium text-muted-foreground">
												{key}
											</p>
											<span className="text-[9px] text-muted-foreground/60">
												{inferValueKind(value).kind}
											</span>
										</div>
										<PropertyValue value={value} propKey={key} />
									</div>
								))}
							</div>
						</div>
					)}
					{propEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t("noPropertiesAvailable", "No properties available")}
						</p>
					)}
					{propEntries.length > 0 && visibleEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t(
								"allFieldsHiddenUseTheFilterToShowThem",
								"All fields hidden — use the filter to show them",
							)}
						</p>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
