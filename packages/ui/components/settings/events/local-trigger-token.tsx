"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRoundIcon, Loader2Icon } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../../lib/error-message";
import { isLocalTriggerDismissal } from "../../../lib/event-save-prompts";
import type { IEvent } from "../../../lib/schema/flow/event";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Banner } from "../devices/primitives/banner";

export { isLocalTriggerDismissal };

/**
 * "Add token": gives this computer's trigger a personal access token without saving the
 * event. `available` is false where the backend registers no triggers on this computer.
 */
export function useRepairLocalTrigger(appId: string) {
	const { t } = useTranslation("settings");
	const { eventState } = useBackend();
	const queryClient = useQueryClient();
	const [pendingId, setPendingId] = useState<string | null>(null);

	const repair = useCallback(
		async (event: IEvent) => {
			if (!eventState.repairLocalTrigger) return;
			setPendingId(event.id);
			try {
				const status = await eventState.repairLocalTrigger(appId, event);
				if (status.active)
					toast.success(
						t("eventNowRunsOnComputer", "{{name}} now runs on this computer.", {
							name: event.name,
						}),
					);
			} catch (error) {
				if (!isLocalTriggerDismissal(error)) {
					console.error(`Failed to add a token to event ${event.id}:`, error);
					toast.error(
						t("couldNotAddToken", "Could not add a token to {{name}}.", {
							name: event.name,
						}),
						{ description: getErrorMessage(error) },
					);
				}
			} finally {
				setPendingId(null);
				await queryClient.invalidateQueries({
					queryKey: ["eventSinkStatus", appId],
				});
			}
		},
		[appId, eventState, queryClient, t],
	);

	return {
		available: typeof eventState.repairLocalTrigger === "function",
		pendingId,
		repair,
	};
}

/** The editor's warning for a trigger this computer registered without a token to sign in with. */
export function LocalTriggerTokenNotice({
	busy,
	onAddToken,
}: Readonly<{ busy: boolean; onAddToken: () => void }>) {
	const { t } = useTranslation("settings");
	const Icon = busy ? Loader2Icon : KeyRoundIcon;
	return (
		<Banner
			tone="warning"
			className="basis-full"
			title={t("triggerNeedsTokenTitle", "Doesn't run on this computer")}
			actions={
				<Button
					variant="outline"
					size="sm"
					disabled={busy}
					onClick={onAddToken}
				>
					<Icon className={cn("h-4 w-4", busy && "animate-spin")} />
					{t("addToken", "Add token")}
				</Button>
			}
		>
			{t(
				"triggerNeedsTokenDetail",
				"Its trigger runs on this computer but has no personal access token to sign in to the hub, so it never fires.",
			)}
		</Banner>
	);
}
