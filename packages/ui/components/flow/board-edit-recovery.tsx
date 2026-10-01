"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Loader2Icon, TriangleAlertIcon } from "lucide-react";
import { useRef, useState } from "react";
import { getErrorMessage } from "../../lib/error-message";
import {
	isBlockingBoardEditJob,
	recoverBoardEditJob,
} from "../../lib/flowpilot/board-edit-job-recovery";
import type { BoardEditJob } from "../../lib/schema/copilot";
import type { AssistantBoardSurface } from "../../state/assistant-surface";
import { useBackend } from "../../state/backend-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";

/** Recovery belongs to the board because the chat that started an edit may be closed. */
export function BoardEditRecovery({
	appId,
	boardId,
	onApplyFlowIrCommit,
}: Readonly<{
	appId: string;
	boardId: string;
	onApplyFlowIrCommit: AssistantBoardSurface["applyFlowIrCommit"];
}>) {
	const { t } = useTranslation("flow");
	const { boardState } = useBackend();
	const [open, setOpen] = useState(false);
	const [busyJobId, setBusyJobId] = useState<string>();
	const busyRef = useRef(false);
	const [messages, setMessages] = useState<Record<string, string>>({});
	const jobs = useQuery({
		queryKey: ["flowpilotBoardEditRecovery", appId, boardId],
		enabled: Boolean(boardState.listBoardEditJobs),
		queryFn: async () => {
			const retained = await boardState.listBoardEditJobs?.(
				appId,
				boardId,
				false,
			);
			return (retained ?? []).filter(
				(job) =>
					job.appId === appId &&
					job.boardId === boardId &&
					isBlockingBoardEditJob(job),
			);
		},
		refetchInterval: 2_000,
	});

	const recover = async (job: BoardEditJob, action: "retry" | "dismiss") => {
		if (busyRef.current) return;
		busyRef.current = true;
		setBusyJobId(job.jobId);
		setMessages((current) => ({ ...current, [job.jobId]: "" }));
		try {
			const result = await recoverBoardEditJob({
				boardState,
				job,
				action,
				replayReceipt: onApplyFlowIrCommit,
			});
			setMessages((current) => ({
				...current,
				[job.jobId]: result.message ?? result.job.error ?? "",
			}));
		} catch (error) {
			setMessages((current) => ({
				...current,
				[job.jobId]: getErrorMessage(error),
			}));
		} finally {
			await jobs.refetch();
			busyRef.current = false;
			setBusyJobId(undefined);
		}
	};

	if (!jobs.data?.length) return null;

	return (
		<>
			<div
				aria-live="polite"
				className="flex shrink-0 flex-wrap items-center gap-2 border-b border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs"
			>
				<TriangleAlertIcon className="size-4 shrink-0 text-amber-600 dark:text-amber-400" />
				<span className="min-w-0 flex-1">
					{t(
						"flowpilotEditBlockingBoard",
						"A FlowPilot edit is blocking changes to this board.",
					)}
				</span>
				<Button size="sm" variant="outline" onClick={() => setOpen(true)}>
					{t("resolveFlowpilotEdit", "Resolve edit")}
				</Button>
			</div>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="sm:max-w-xl">
					<DialogHeader>
						<DialogTitle>
							{t("recoverFlowpilotEdits", "Recover FlowPilot edits")}
						</DialogTitle>
						<DialogDescription>
							{t(
								"recoverFlowpilotEditsDescription",
								"Resolve these edits to continue changing the board. Recovery remains available after closing FlowPilot.",
							)}
						</DialogDescription>
					</DialogHeader>
					{jobs.data.map((job) => (
						<section
							key={job.jobId}
							className="space-y-3 rounded-lg border p-3"
						>
							<p className="text-sm font-medium">
								{job.phase === "applied_pending_delivery"
									? t(
											"flowpilotEditNeedsSync",
											"Changes saved, sync unfinished",
										)
									: job.phase === "applying"
										? t(
												"flowpilotEditApplying",
												"Apply in progress or interrupted",
											)
										: t("flowpilotEditFailed", "Apply needs recovery")}
							</p>
							<p className="text-xs text-muted-foreground">
								{job.phase === "applied_pending_delivery"
									? t(
											"flowpilotFinishSavedEdit",
											"Finish syncing the saved changes to unlock the board.",
										)
									: job.phase === "applying"
										? t(
												"flowpilotRecoverApplyingEdit",
												"Recovery waits for any active apply and checks whether these changes were already saved.",
											)
										: t(
												"flowpilotRecoverFailedEdit",
												"Retry the reviewed changes or dismiss the unapplied edit. Changes already saved will finish syncing.",
											)}
							</p>
							{(messages[job.jobId] || job.error) && (
								<p
									role="alert"
									className="text-xs text-destructive wrap-anywhere"
								>
									{messages[job.jobId] || job.error}
								</p>
							)}
							{job.phase !== "applied_pending_delivery" && (
								<div className="space-y-2 text-xs">
									<p>
										{t("flowpilotRecoveryChangeCount", {
											defaultValue_one: "{{count}} reviewed change",
											defaultValue_other: "{{count}} reviewed changes",
											count: job.review.commandCount,
										})}
									</p>
									<ul className="max-h-40 space-y-1 overflow-y-auto pl-4 list-disc wrap-anywhere">
										{job.review.commandSummaries.map((summary, index) => (
											<li key={`${index}:${summary}`}>{summary}</li>
										))}
									</ul>
									{(job.review.replacementMode ||
										job.review.destructiveEffects.length > 0) && (
										<div className="space-y-1 rounded-md bg-destructive/10 p-2 text-destructive">
											<p>
												{t(
													"flowpilotRecoveryDestructive",
													"These changes can remove or replace board content.",
												)}
											</p>
											{job.review.destructiveEffects.map((effect, index) => (
												<p key={`${index}:${effect}`} className="wrap-anywhere">
													{effect}
												</p>
											))}
										</div>
									)}
								</div>
							)}
							<div className="flex flex-wrap justify-end gap-2">
								{job.phase === "failed" && (
									<Button
										variant="outline"
										size="sm"
										disabled={Boolean(busyJobId)}
										onClick={() => void recover(job, "dismiss")}
									>
										{t("dismissFlowpilotEdit", "Dismiss edit")}
									</Button>
								)}
								<Button
									size="sm"
									disabled={Boolean(busyJobId)}
									onClick={() => void recover(job, "retry")}
								>
									{busyJobId === job.jobId && (
										<Loader2Icon className="size-3 animate-spin" />
									)}
									{job.phase === "applied_pending_delivery"
										? t("finishFlowpilotSync", "Finish sync")
										: job.phase === "applying"
											? t("recoverFlowpilotEdit", "Recover edit")
											: t("retryFlowpilotApply", "Retry apply")}
								</Button>
							</div>
						</section>
					))}
				</DialogContent>
			</Dialog>
		</>
	);
}
