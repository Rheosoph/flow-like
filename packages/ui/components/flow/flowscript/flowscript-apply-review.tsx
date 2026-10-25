"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import type { ApplyImpact } from "./flowscript-apply-preview";

export interface FlowScriptApplyReview {
	impact?: ApplyImpact;
	/** Older hosts may return only a diagnostic, without a command plan. */
	diagnostic?: string;
}

export function FlowScriptApplyReviewDialog({
	review,
	onClose,
	onConfirm,
}: Readonly<{
	review?: FlowScriptApplyReview;
	onClose: () => void;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("flow");
	const deletesId = useId();
	const movesId = useId();
	const impact = review?.impact;
	const actionLabel = impact?.moves.length
		? impact.deletions.length
			? t("flowscriptApplyChanges", "Apply changes")
			: t("flowscriptApplyWithMoves", "Apply with moves")
		: t("applyWithDeletions", "Apply with deletions");

	return (
		<AlertDialog
			open={review !== undefined}
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<AlertDialogContent className="flex max-h-[85vh] flex-col sm:max-w-2xl">
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("flowscriptApplyReviewTitle", "Review deletions and moves")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"flowscriptApplyReviewDescription",
							"Review the board items this FlowScript will delete or move before applying.",
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<div className="min-h-0 space-y-4 overflow-y-auto text-sm">
					{impact ? (
						<>
							<section aria-labelledby={deletesId}>
								<h3
									id={deletesId}
									className="mb-2 font-semibold text-destructive"
								>
									{t("flowscriptApplyDeleteCount", {
										defaultValue: "Deletes ({{count}})",
										count: impact.deletions.length,
									})}
								</h3>
								{impact.deletions.length ? (
									<ul
										aria-labelledby={deletesId}
										className="max-h-48 space-y-2 overflow-y-auto rounded-md border p-3"
									>
										{impact.deletions.map((item) => (
											<li key={item.key} className="break-words">
												<span>{item.label}</span>
												{item.id && item.id !== item.label && (
													<code className="block break-all text-xs text-muted-foreground">
														{item.id}
													</code>
												)}
											</li>
										))}
									</ul>
								) : (
									<p className="text-muted-foreground">
										{t("flowscriptApplyNoDeletes", "No items will be deleted.")}
									</p>
								)}
							</section>
							<section aria-labelledby={movesId}>
								<h3 id={movesId} className="mb-2 font-semibold">
									{t("flowscriptApplyMoveCount", {
										defaultValue: "Moves ({{count}})",
										count: impact.moves.length,
									})}
								</h3>
								{impact.moves.length ? (
									<ul
										aria-labelledby={movesId}
										className="max-h-48 space-y-2 overflow-y-auto rounded-md border p-3"
									>
										{impact.moves.map((item) => (
											<li key={item.key} className="break-words">
												<span>{item.label}</span>
												{item.id !== item.label && (
													<code className="block break-all text-xs text-muted-foreground">
														{item.id}
													</code>
												)}
												<span className="block text-xs text-muted-foreground">
													{t("flowscriptApplyMoveTarget", {
														defaultValue: "To: {{target}}",
														target:
															item.targetLayer ||
															t("flowscriptApplyBoardRoot", "Board root"),
													})}
												</span>
											</li>
										))}
									</ul>
								) : (
									<p className="text-muted-foreground">
										{t("flowscriptApplyNoMoves", "No items will be moved.")}
									</p>
								)}
							</section>
						</>
					) : (
						<p className="break-words text-muted-foreground">
							{review?.diagnostic}
						</p>
					)}
				</div>
				<AlertDialogFooter>
					<AlertDialogCancel>
						{t("keepEverything", "Keep everything")}
					</AlertDialogCancel>
					<AlertDialogAction onClick={onConfirm}>
						{actionLabel}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
