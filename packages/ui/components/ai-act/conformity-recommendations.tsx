"use client";

import { Lightbulb, Pencil, ShieldCheck, TrendingUp } from "lucide-react";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui";

/** A single weighted conformity-improvement tip from the EU AI Act classifier. */
export interface Recommendation {
	id: string;
	title: string;
	detail: string;
	category: string;
	potentialPoints: number;
	article?: string | null;
	answerKey?: string | null;
}

export const RECOMMENDATION_CATEGORY_STYLES: Record<string, string> = {
	Transparency: "text-blue-500",
	Board: "text-violet-500",
	Oversight: "text-amber-500",
	Data: "text-cyan-500",
	Posture: "text-emerald-500",
	Classification: "text-red-500",
};

export interface ConformityRecommendationsProps {
	recommendations: Recommendation[];
	/** Current conformity score (0–100) used to compute the projected uplift. */
	score: number | null;
	/** When provided, renders an "Edit assessment" action button. */
	onEdit?: () => void;
	className?: string;
}

/**
 * Shared, read-only presentation of the conformity improvement tips. Used by
 * both the admin governance inventory and the owner publishing wizard so the
 * guidance and estimated uplift are always identical.
 */
export function ConformityRecommendations({
	recommendations,
	score,
	onEdit,
	className,
}: Readonly<ConformityRecommendationsProps>) {
	if (!recommendations || recommendations.length === 0) {
		return (
			<Card
				className={`border-emerald-500/30 bg-emerald-500/5 ${className ?? ""}`}
			>
				<CardHeader>
					<CardTitle className="text-sm flex items-center gap-2">
						<ShieldCheck className="h-4 w-4 text-emerald-500" />
						No conformity actions outstanding
					</CardTitle>
					<CardDescription>
						This assessment scores full marks on every weighted factor. Keep the
						board scores and attached-model registry up to date to maintain it.
					</CardDescription>
				</CardHeader>
			</Card>
		);
	}

	const totalUplift = recommendations.reduce(
		(sum, rec) => sum + Math.max(0, rec.potentialPoints),
		0,
	);
	const projected =
		typeof score === "number" ? Math.min(100, score + totalUplift) : null;

	return (
		<Card className={className}>
			<CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
				<div>
					<CardTitle className="text-sm flex items-center gap-2">
						<Lightbulb className="h-4 w-4 text-amber-500" />
						How to improve the conformity score
					</CardTitle>
					<CardDescription>
						Prioritised, actionable steps. Estimated uplift mirrors the exact
						weighted scoring model.
					</CardDescription>
				</div>
				{projected !== null && totalUplift > 0 && (
					<div className="flex items-center gap-2 rounded-lg border bg-muted/40 px-3 py-2 text-xs">
						<TrendingUp className="h-4 w-4 text-emerald-500" />
						<span className="text-muted-foreground">Potential</span>
						<span className="font-semibold">{score}</span>
						<span className="text-muted-foreground">→</span>
						<span className="font-bold text-emerald-600 dark:text-emerald-400">
							{projected}
						</span>
					</div>
				)}
			</CardHeader>
			<CardContent className="space-y-2">
				{recommendations.map((rec) => (
					<div
						key={rec.id}
						className="flex items-start gap-3 rounded-lg border p-3"
					>
						<div className="mt-0.5 flex h-10 w-12 shrink-0 flex-col items-center justify-center rounded-md border bg-muted/40">
							<span className="text-sm font-bold text-emerald-600 dark:text-emerald-400">
								+{Math.max(0, rec.potentialPoints)}
							</span>
							<span className="text-[9px] uppercase tracking-wide text-muted-foreground">
								pts
							</span>
						</div>
						<div className="min-w-0 flex-1">
							<div className="flex flex-wrap items-center gap-2">
								<p className="text-sm font-medium">{rec.title}</p>
								<Badge
									variant="outline"
									className={`text-[10px] ${
										RECOMMENDATION_CATEGORY_STYLES[rec.category] ?? ""
									}`}
								>
									{rec.category}
								</Badge>
								{rec.article && (
									<Badge variant="secondary" className="text-[10px]">
										{rec.article}
									</Badge>
								)}
							</div>
							<p className="mt-1 text-xs text-muted-foreground">{rec.detail}</p>
						</div>
					</div>
				))}
				{onEdit && (
					<div className="flex justify-end pt-1">
						<Button variant="outline" size="sm" onClick={onEdit}>
							<Pencil className="mr-2 h-4 w-4" />
							Edit assessment
						</Button>
					</div>
				)}
			</CardContent>
		</Card>
	);
}
