"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { AtSign, Loader2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../lib/api-error";
import type { IProfile } from "../../../lib/schema/profile/profile";
import { useBackend } from "../../../state/backend-state";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";

/** What `POST /admin/users/repair-identities` changed, or would change, in one page. */
interface IdentityRepairTotals {
	scanned: number;
	handles_set: number;
	handles_cleared: number;
	names_cleared: number;
	skipped: number;
}

interface IdentityRepairPage extends IdentityRepairTotals {
	next_cursor?: string | null;
}

const NOTHING: IdentityRepairTotals = {
	scanned: 0,
	handles_set: 0,
	handles_cleared: 0,
	names_cleared: 0,
	skipped: 0,
};

const PAGE_SIZE = "200";

function add(
	totals: IdentityRepairTotals,
	page: IdentityRepairPage,
): IdentityRepairTotals {
	return {
		scanned: totals.scanned + page.scanned,
		handles_set: totals.handles_set + page.handles_set,
		handles_cleared: totals.handles_cleared + page.handles_cleared,
		names_cleared: totals.names_cleared + page.names_cleared,
		skipped: totals.skipped + page.skipped,
	};
}

/**
 * Removes email addresses and account ids from the names and usernames of every
 * account, after showing what a dry run would change. Accounts get a username
 * derived from their name or address; usernames people chose stay.
 */
export function UserIdentityRepair({
	profile,
}: Readonly<{ profile?: IProfile }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const client = useQueryClient();
	const [open, setOpen] = useState(false);
	const [preview, setPreview] = useState<IdentityRepairTotals>();
	const [running, setRunning] = useState<"preview" | "apply">();
	const [error, setError] = useState<string>();

	async function pass(dryRun: boolean): Promise<IdentityRepairTotals> {
		if (!profile) throw new Error("Your hub profile is still loading.");
		let totals = NOTHING;
		let after: string | undefined;
		do {
			const query = new URLSearchParams({
				dry_run: String(dryRun),
				limit: PAGE_SIZE,
			});
			if (after) query.set("after", after);
			const page = await backend.apiState.post<IdentityRepairPage>(
				profile,
				`admin/users/repair-identities?${query}`,
			);
			totals = add(totals, page);
			after = page.next_cursor ?? undefined;
		} while (after);
		return totals;
	}

	async function start() {
		setOpen(true);
		setPreview(undefined);
		setError(undefined);
		setRunning("preview");
		try {
			setPreview(await pass(true));
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t(
						"identityRepairPreviewFailed",
						"The accounts could not be checked.",
					),
				),
			);
		} finally {
			setRunning(undefined);
		}
	}

	async function apply() {
		setRunning("apply");
		setError(undefined);
		try {
			const done = await pass(false);
			setOpen(false);
			await client.invalidateQueries({ queryKey: ["admin", "users"] });
			toast.success(
				t("identityRepairDone", {
					defaultValue:
						"{{handles}} accounts got a username and {{names}} names no longer show an email address.",
					handles: done.handles_set,
					names: done.names_cleared,
				}),
			);
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t(
						"identityRepairFailed",
						"Not every account could be repaired. Run it again to continue where it stopped.",
					),
				),
			);
		} finally {
			setRunning(undefined);
		}
	}

	const changes = preview
		? preview.handles_set + preview.handles_cleared + preview.names_cleared
		: 0;

	return (
		<>
			<Button
				variant="outline"
				size="sm"
				disabled={!profile}
				onClick={() => void start()}
			>
				<AtSign className="mr-2 h-4 w-4" />
				{t("identityRepair", "Fix usernames")}
			</Button>
			<AlertDialog
				open={open}
				onOpenChange={(next) => {
					if (!running) setOpen(next);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("identityRepairTitle", "Remove email addresses from names")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"identityRepairDescription",
								"Accounts whose name or username is an email address or an account id get a username derived from their name or address. Usernames people chose stay. Accounts with nothing to derive from choose one at their next sign-in.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<div aria-live="polite" className="space-y-1 text-sm">
						{running === "preview" && (
							<p className="flex items-center gap-2 text-muted-foreground">
								<Loader2 className="h-4 w-4 animate-spin" />
								{t("identityRepairChecking", "Checking every account...")}
							</p>
						)}
						{preview && (
							<>
								<p>
									{t("identityRepairPreviewHandles", {
										defaultValue: "{{count}} accounts get a username.",
										count: preview.handles_set,
									})}
								</p>
								<p>
									{t("identityRepairPreviewNames", {
										defaultValue:
											"{{count}} names no longer show an email address.",
										count: preview.names_cleared,
									})}
								</p>
								<p className="text-muted-foreground">
									{t("identityRepairPreviewCleared", {
										defaultValue:
											"{{count}} accounts lose an address or id as username and choose one at their next sign-in.",
										count: preview.handles_cleared,
									})}
								</p>
								<p className="text-muted-foreground">
									{t("identityRepairPreviewScanned", {
										defaultValue: "{{count}} accounts checked.",
										count: preview.scanned,
									})}
								</p>
							</>
						)}
						{error && (
							<p role="alert" className="text-destructive">
								{error}
							</p>
						)}
					</div>
					<AlertDialogFooter>
						<AlertDialogCancel disabled={Boolean(running)}>
							{t("cancel", "Cancel")}
						</AlertDialogCancel>
						<Button
							disabled={Boolean(running) || !preview || changes === 0}
							onClick={() => void apply()}
						>
							{running === "apply" && (
								<Loader2 className="mr-2 h-4 w-4 animate-spin" />
							)}
							{t("identityRepairApply", "Repair accounts")}
						</Button>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}
