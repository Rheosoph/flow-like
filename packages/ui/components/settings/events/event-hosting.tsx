"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	Copy,
	ExternalLink,
	Loader2,
	Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { getApiOrigin } from "../../../lib/api-url";
import {
	getHostedFrontendKind,
	getHostedFrontendUrl,
	parseFrontendHosting,
} from "../../../lib/frontend-hosting";
import {
	type IEvent,
	IEventExecutionMode,
	IEventExposure,
} from "../../../lib/schema/flow/event";
import type { IFrontendHosting } from "../../../lib/schema/flow/event-payload";
import { useBackend } from "../../../state/backend-state";
import type { IEventAlias } from "../../../state/backend-state/event-state";
import { Alert, AlertDescription } from "../../ui/alert";
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
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";

async function listNoEventAliases(): Promise<IEventAlias[]> {
	return [];
}

export function EventHosting({
	appId,
	event,
	config,
	canWrite,
	hasUnsavedChanges,
	onUpdate,
}: {
	appId: string;
	event: IEvent;
	config: Record<string, unknown>;
	canWrite: boolean;
	hasUnsavedChanges: boolean;
	onUpdate: (hosting: IFrontendHosting) => void;
}) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const aliases = useInvoke<IEventAlias[], [string, string]>(
		backend.eventState.listEventAliases ?? listNoEventAliases,
		backend.eventState,
		[appId, event.id],
		Boolean(appId && event.id && backend.eventState.listEventAliases),
	);
	const [aliasInput, setAliasInput] = useState("");
	const [aliasBusy, setAliasBusy] = useState(false);
	const [aliasError, setAliasError] = useState<string | null>(null);
	const [confirmAnonymous, setConfirmAnonymous] = useState(false);
	const currentAlias = aliases.data?.[0]?.slug ?? "";
	useEffect(() => {
		setAliasInput(currentAlias);
		setAliasError(null);
	}, [currentAlias]);

	const kind = getHostedFrontendKind(event);
	if (!kind) return null;
	const hosting = parseFrontendHosting(config);
	const anonymousWarning = t(
		"hostingAnonymousWarning",
		"Anyone with this link can run the workflow without signing in. The app owner pays for all anonymous usage, including compute and model costs. Only enable this for content and actions you intend to make public.",
	);
	const origin = getApiOrigin(profile.data);
	const directUrl = getHostedFrontendUrl(origin, kind, event.id);
	const aliasUrl = currentAlias
		? getHostedFrontendUrl(origin, kind, currentAlias)
		: null;
	const available =
		hosting.enabled &&
		event.active &&
		event.execution_mode === IEventExecutionMode.Remote &&
		(event.exposure ?? IEventExposure.Public) === IEventExposure.Public;
	const aliasDisabled =
		!canWrite || aliasBusy || hasUnsavedChanges || aliases.isLoading;

	const saveAlias = async () => {
		const slug = aliasInput.trim().toLowerCase();
		if (aliasDisabled || !slug || !backend.eventState.upsertEventAlias) return;
		setAliasBusy(true);
		setAliasError(null);
		try {
			await backend.eventState.upsertEventAlias(appId, event.id, slug);
			await aliases.refetch();
			toast.success(t("hostingAliasSaved", "Alias saved"));
		} catch (error) {
			setAliasError(
				error instanceof Error
					? error.message
					: t("hostingAliasSaveFailed", "Failed to save alias"),
			);
		} finally {
			setAliasBusy(false);
		}
	};
	const deleteAlias = async () => {
		if (aliasDisabled || !currentAlias || !backend.eventState.deleteEventAlias)
			return;
		setAliasBusy(true);
		setAliasError(null);
		try {
			await backend.eventState.deleteEventAlias(appId, event.id, currentAlias);
			await aliases.refetch();
			toast.success(t("hostingAliasRemoved", "Alias removed"));
		} catch (error) {
			setAliasError(
				error instanceof Error
					? error.message
					: t("hostingAliasRemoveFailed", "Failed to remove alias"),
			);
		} finally {
			setAliasBusy(false);
		}
	};
	const copyUrl = async (url: string) => {
		try {
			await navigator.clipboard.writeText(url);
			toast.success(t("hostingLinkCopied", "Link copied"));
		} catch {
			toast.error(t("hostingLinkCopyFailed", "Could not copy the link"));
		}
	};
	const urlField = (id: string, label: string, url: string) => (
		<div className="space-y-2">
			<Label htmlFor={id}>{label}</Label>
			<div className="flex items-center gap-2">
				<Input
					id={id}
					readOnly
					value={url}
					className="min-w-0 font-mono text-xs"
				/>
				<Button
					type="button"
					variant="outline"
					size="icon"
					onClick={() => copyUrl(url)}
					aria-label={t("hostingCopyLink", "Copy link")}
				>
					<Copy className="size-4" />
				</Button>
				<Button
					type="button"
					variant="outline"
					size="icon"
					asChild
					disabled={!available || hasUnsavedChanges}
				>
					<a
						href={available && !hasUnsavedChanges ? url : undefined}
						target="_blank"
						rel="noopener noreferrer"
						aria-label={t("hostingOpenPage", "Open hosted page")}
						aria-disabled={!available || hasUnsavedChanges}
					>
						<ExternalLink className="size-4" />
					</a>
				</Button>
			</div>
		</div>
	);

	return (
		<div className="space-y-6 rounded-lg border p-5">
			<AlertDialog
				open={confirmAnonymous && hosting.enabled && canWrite}
				onOpenChange={setConfirmAnonymous}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("hostingAnonymousConfirmTitle", "Allow anonymous access?")}
						</AlertDialogTitle>
						<AlertDialogDescription>{anonymousWarning}</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction
							disabled={!canWrite || !hosting.enabled}
							onClick={() => {
								if (!canWrite || !hosting.enabled) return;
								onUpdate({
									...hosting,
									allow_anonymous: true,
									auth_proxy: false,
								});
							}}
						>
							{t("hostingAnonymousConfirm", "Enable anonymous access")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
			<div className="flex items-start justify-between gap-4">
				<div className="space-y-1">
					<Label htmlFor="frontend-hosting-enabled">
						{t("hostingEnable", "Enable static hosting")}
					</Label>
					<p className="text-sm text-muted-foreground">
						{t(
							"hostingEnableDescription",
							"Publish a direct link to this chat, form or page. Visitors must sign in unless you separately allow anonymous access.",
						)}
					</p>
				</div>
				<Switch
					id="frontend-hosting-enabled"
					checked={hosting.enabled}
					disabled={!canWrite}
					onCheckedChange={(enabled) => {
						setConfirmAnonymous(false);
						onUpdate({ enabled, allow_anonymous: false, auth_proxy: true });
					}}
				/>
			</div>
			<div className="flex items-start justify-between gap-4">
				<div className="space-y-1">
					<Label htmlFor="frontend-hosting-anonymous">
						{t("hostingAnonymous", "Allow anonymous access")}
					</Label>
					<p className="text-sm text-muted-foreground">
						{t(
							"hostingAnonymousDescription",
							"Off by default. Enabling this removes sign-in and charges anonymous runs to the app owner.",
						)}
					</p>
				</div>
				<Switch
					id="frontend-hosting-anonymous"
					checked={hosting.allow_anonymous}
					disabled={!canWrite || !hosting.enabled}
					onCheckedChange={(allowAnonymous) => {
						if (allowAnonymous) {
							setConfirmAnonymous(true);
							return;
						}
						onUpdate({ ...hosting, allow_anonymous: false, auth_proxy: true });
					}}
				/>
			</div>
			{hosting.allow_anonymous ? (
				<Alert variant="destructive">
					<AlertTriangle className="size-4" />
					<AlertDescription>{anonymousWarning}</AlertDescription>
				</Alert>
			) : (
				<p className="text-sm text-muted-foreground">
					{t(
						"hostingSignInDefault",
						"Hosted visitors sign in through Flow-Like and return to this page. The workflow receives their verified identity.",
					)}
				</p>
			)}
			{hosting.enabled && !available && (
				<Alert>
					<AlertDescription>
						{t(
							"hostingRequirements",
							"Set execution to Remote in the event or its workflow, activate the event and select Public exposure. Save these settings before sharing the link.",
						)}
					</AlertDescription>
				</Alert>
			)}
			{urlField(
				"frontend-hosting-url",
				t("hostingDirectLink", "Direct link"),
				directUrl,
			)}
			<div className="space-y-2">
				<Label htmlFor="frontend-hosting-alias">
					{t("hostingAlias", "Public alias")}
				</Label>
				<div className="flex flex-wrap items-center gap-2">
					<span className="text-sm text-muted-foreground">/{kind}/</span>
					<Input
						id="frontend-hosting-alias"
						className="min-w-40 flex-1"
						value={aliasInput}
						placeholder="my-assistant"
						disabled={aliasDisabled || !backend.eventState.upsertEventAlias}
						onChange={(e) => setAliasInput(e.target.value)}
					/>
					<Button
						type="button"
						variant="outline"
						disabled={
							aliasDisabled ||
							!backend.eventState.upsertEventAlias ||
							!aliasInput.trim() ||
							aliasInput.trim().toLowerCase() === currentAlias
						}
						onClick={saveAlias}
					>
						{aliasBusy && <Loader2 className="mr-2 size-4 animate-spin" />}
						{t("hostingSaveAlias", "Save alias")}
					</Button>
					{currentAlias && (
						<Button
							type="button"
							variant="outline"
							size="icon"
							disabled={aliasDisabled || !backend.eventState.deleteEventAlias}
							onClick={deleteAlias}
							aria-label={t("hostingRemoveAlias", "Remove alias")}
						>
							<Trash2 className="size-4" />
						</Button>
					)}
				</div>
				<p className="text-xs text-muted-foreground">
					{hasUnsavedChanges
						? t(
								"hostingSaveBeforeAlias",
								"Save event changes before updating the alias.",
							)
						: t(
								"hostingAliasImmediate",
								"Alias changes take effect immediately. Changing an alias stops the previous link from working.",
							)}
				</p>
				{(aliasError || aliases.error) && (
					<p role="alert" className="text-sm text-destructive">
						{aliasError ?? aliases.error?.message}
					</p>
				)}
			</div>
			{aliasUrl &&
				urlField(
					"frontend-hosting-alias-url",
					t("hostingAliasLink", "Alias link"),
					aliasUrl,
				)}
		</div>
	);
}
