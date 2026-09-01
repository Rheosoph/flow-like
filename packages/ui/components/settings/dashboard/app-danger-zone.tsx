"use client";

import { useTranslation } from "@flow-like/locales";
import { BombIcon, LogOutIcon } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks";
import type { IOwnRole } from "../../../state/backend-state";
import { useBackend } from "../../../state/backend-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { VerificationDialog } from "../../verification-dialog";

export interface DangerActions {
	canDelete: boolean;
	canLeave: boolean;
}

/**
 * Which ways out of a project this account actually has.
 *
 * Both answers come from the hub rather than being inferred here, because the
 * two use different predicates and guessing either one wrong is expensive:
 * deleting needs an `Owner` check that `Admin` also satisfies, while leaving
 * needs the `Owner` bit to be *absent* — the team endpoint refuses to remove an
 * owner's membership. An admin who is not an owner legitimately has both.
 *
 * A role that could not be read yields neither. Falling back to "assume owner"
 * would put a Delete button under the cursor of someone who cannot use it.
 */
export function resolveDangerActions(
	ownRole: IOwnRole | undefined,
	canEdit: boolean,
): DangerActions {
	if (!ownRole) return { canDelete: false, canLeave: false };
	return {
		canDelete: canEdit && ownRole.is_owner,
		canLeave: ownRole.can_leave,
	};
}

/**
 * The one place a project can be given up, showing only what this account may
 * actually do. See {@link resolveDangerActions} for how that is decided.
 */
export function AppDangerZone({
	appId,
	canEdit,
	onDeleted,
	onLeft,
}: Readonly<{
	appId: string;
	canEdit: boolean;
	onDeleted: () => Promise<void> | void;
	onLeft: () => Promise<void> | void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [busy, setBusy] = useState(false);

	const ownRole = useInvoke(
		backend.roleState.getOwnRole,
		backend.roleState,
		[appId],
		appId.length > 0,
	);

	const run = useCallback(
		async (action: () => Promise<void> | void, fallback: string) => {
			setBusy(true);
			try {
				await action();
			} catch (error) {
				toast.error(error instanceof Error ? error.message : fallback);
			} finally {
				setBusy(false);
			}
		},
		[],
	);

	if (ownRole.isPending) {
		return <Skeleton className="h-28 w-full rounded-lg" />;
	}

	if (!ownRole.data) {
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"couldNotCheckYourAccessToThisProject",
					"Could not check your access to this project.",
				)}
			</p>
		);
	}

	const { canDelete, canLeave } = resolveDangerActions(ownRole.data, canEdit);

	return (
		<div className="space-y-3">
			{canDelete && (
				<div className="space-y-2 rounded-lg border border-destructive/40 p-4">
					<h4 className="text-sm font-semibold text-destructive">
						{t("deleteThisApp", "Delete this app")}
					</h4>
					<p className="text-xs text-muted-foreground">
						{t(
							"removesEveryFlowEventPageAndStoredFileThisCannotBeUndone",
							"Removes every flow, event, page and stored file. This cannot be undone.",
						)}
					</p>
					<VerificationDialog
						dialog="You cannot undo this action. This will permanently delete the app!"
						onConfirm={() => {
							void run(
								onDeleted,
								t("couldNotDeleteTheApp", "Could not delete the app"),
							);
						}}
					>
						<Button variant="destructive" size="sm" disabled={busy}>
							<BombIcon className="mr-1.5 h-3 w-3" />
							{t("deleteApp", "Delete app")}
						</Button>
					</VerificationDialog>
				</div>
			)}

			{canLeave && (
				<div className="space-y-2 rounded-lg border p-4">
					<h4 className="text-sm font-semibold">
						{t("quitThisProject", "Quit this project")}
					</h4>
					<p className="text-xs text-muted-foreground">
						{t(
							"youLoseAccessAndItLeavesYourLibraryTheProjectItselfAndEveryoneElsesAccessToItAreUntouched",
							"You lose access and it leaves your library. The project itself, and everyone else's access to it, are untouched — an owner can invite you back.",
						)}
					</p>
					<AlertDialog>
						<AlertDialogTrigger asChild>
							<Button
								variant="outline"
								size="sm"
								disabled={busy}
								className="text-destructive hover:text-destructive"
							>
								<LogOutIcon className="mr-1.5 h-3 w-3" />
								{t("quitProject", "Quit project")}
							</Button>
						</AlertDialogTrigger>
						<AlertDialogContent>
							<AlertDialogHeader>
								<AlertDialogTitle>
									{t("quitThisProject2", "Quit this project?")}
								</AlertDialogTitle>
								<AlertDialogDescription>
									{t(
										"yourMembershipEndsImmediatelyAndTheLocalCopyIsRemovedYouWillNeedANewInviteToReturn",
										"Your membership ends immediately and the local copy is removed. You will need a new invite to return.",
									)}
								</AlertDialogDescription>
							</AlertDialogHeader>
							<AlertDialogFooter>
								<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
								<AlertDialogAction
									className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
									onClick={() => {
										void run(
											onLeft,
											t("couldNotQuitTheProject", "Could not quit the project"),
										);
									}}
								>
									{t("quitProject", "Quit project")}
								</AlertDialogAction>
							</AlertDialogFooter>
						</AlertDialogContent>
					</AlertDialog>
				</div>
			)}

			{!canLeave && (
				<p className="text-xs text-muted-foreground">
					{canDelete
						? t(
								"anOwnerCannotQuitTheirOwnProjectHandOverOwnershipFirstOrDeleteIt",
								"An owner cannot quit their own project — hand ownership over first, or delete it.",
							)
						: t(
								"nothingToDoHereThisProjectCannotBeDeletedOrQuitFromThisAccount",
								"Nothing to do here — this project cannot be deleted or quit from this account.",
							)}
				</p>
			)}
		</div>
	);
}
