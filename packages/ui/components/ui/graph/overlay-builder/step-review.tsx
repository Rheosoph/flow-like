"use client";

import { AlertCircle, CheckCircle2 } from "lucide-react";
import type {
	EdgeLabelMapping,
	NodeLabelMapping,
	ValidationResult,
} from "../../../../state/backend-state/graph-state";
import { Badge } from "../../badge";
import { Card } from "../../card";
import { getGraphIcon } from "../icons";

export interface StepReviewProps {
	name: string;
	description: string;
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	defaultLimit: number;
	validation: ValidationResult | null;
}

export function StepReview({
	name,
	description,
	nodes,
	edges,
	defaultLimit,
	validation,
}: StepReviewProps) {
	return (
		<div className="space-y-4">
			<div>
				<h3 className="text-sm font-medium mb-1">Review Overlay</h3>
				<p className="text-xs text-muted-foreground">
					Review your graph overlay configuration before creating it.
				</p>
			</div>

			<Card className="p-4 space-y-2">
				<div className="flex items-center justify-between">
					<h4 className="text-sm font-semibold">
						{name || "Untitled Overlay"}
					</h4>
					<Badge variant="secondary" className="text-[10px]">
						Limit: {defaultLimit}
					</Badge>
				</div>
				{description && (
					<p className="text-xs text-muted-foreground">{description}</p>
				)}
			</Card>

			{nodes.length > 0 && (
				<div className="space-y-2">
					<p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
						Node Labels ({nodes.length})
					</p>
					<div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
						{nodes.map((n) => {
							const Icon = getGraphIcon(n.style.icon);
							return (
								<div
									key={n.label}
									className="flex items-center gap-2 rounded-lg border p-2"
								>
									<div
										className="w-5 h-5 rounded-full flex items-center justify-center shrink-0"
										style={{ backgroundColor: n.style.color }}
									>
										<Icon className="h-3 w-3 text-white" />
									</div>
									<div className="min-w-0">
										<p className="text-xs font-medium truncate">{n.label}</p>
										<p className="text-[10px] text-muted-foreground truncate">
											{n.table} → {n.id_column}
										</p>
									</div>
								</div>
							);
						})}
					</div>
				</div>
			)}

			{edges.length > 0 && (
				<div className="space-y-2">
					<p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
						Edge Labels ({edges.length})
					</p>
					<div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
						{edges.map((e) => (
							<div
								key={e.label}
								className="flex items-center gap-2 rounded-lg border p-2"
							>
								<div
									className="w-5 h-1 rounded shrink-0"
									style={{ backgroundColor: e.style.color }}
								/>
								<div className="min-w-0">
									<p className="text-xs font-medium truncate">{e.label}</p>
									<p className="text-[10px] text-muted-foreground truncate">
										{e.src_label} → {e.dst_label} ({e.table})
									</p>
								</div>
							</div>
						))}
					</div>
				</div>
			)}

			{validation && (
				<div
					className={`rounded-lg border p-3 ${validation.ok ? "bg-green-500/10 border-green-500/30" : "bg-destructive/10 border-destructive/30"}`}
				>
					<div className="flex items-center gap-2 mb-1">
						{validation.ok ? (
							<CheckCircle2 className="h-4 w-4 text-green-500" />
						) : (
							<AlertCircle className="h-4 w-4 text-destructive" />
						)}
						<span className="text-xs font-medium">
							{validation.ok ? "Validation passed" : "Validation issues found"}
						</span>
					</div>
					{validation.issues.length > 0 && (
						<ul className="space-y-1 mt-2">
							{validation.issues.map((issue, i) => (
								<li
									key={i}
									className="text-xs text-muted-foreground flex items-start gap-1.5"
								>
									<span className="text-destructive mt-0.5">•</span>
									{issue}
								</li>
							))}
						</ul>
					)}
				</div>
			)}
		</div>
	);
}
