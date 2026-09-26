"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangle, Copy, ExternalLink } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { getApiOrigin } from "../../../lib/api-url";
import {
	HOSTING_BLOCKER_FIX,
	type HostingBlocker,
	getHostedFrontendKind,
	getHostedFrontendUrl,
	getHostingBlockers,
	parseFrontendHosting,
} from "../../../lib/frontend-hosting";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IFrontendHosting } from "../../../lib/schema/flow/event-payload";
import { useBackend } from "../../../state/backend-state";
import { Alert, AlertDescription, AlertTitle } from "../../ui/alert";
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
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";

/**
 * A hosted app is one link, `/a/{appId}`; each routed chat, form or page is
 * published on its own route beneath it.
 */
export function EventHosting({
	appId,
	event,
	route,
	config,
	canWrite,
	hasUnsavedChanges,
	onUpdate,
	onEventChange,
}: {
	appId: string;
	event: IEvent;
	/** The saved route this Event answers, or `null` when it has none. */
	route: string | null;
	config: Record<string, unknown>;
	canWrite: boolean;
	hasUnsavedChanges: boolean;
	onUpdate: (hosting: IFrontendHosting) => void;
	onEventChange?: (
		change: Partial<Pick<IEvent, "active" | "execution_mode" | "exposure">>,
	) => void;
}) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const [confirmAnonymous, setConfirmAnonymous] = useState(false);

	if (!getHostedFrontendKind(event)) return null;
	const hosting = parseFrontendHosting(config);
	const anonymousWarning = t(
		"hostingAnonymousWarning",
		"Anyone with this link can run the workflow without signing in. The app owner pays for all anonymous usage, including compute and model costs. Only enable this for content and actions you intend to make public.",
	);
	const origin = getApiOrigin(profile.data);
	const url =
		route === null ? null : getHostedFrontendUrl(origin, appId, route);
	const blockers = getHostingBlockers(event, route);
	const available = hosting.enabled && blockers.length === 0;
	const live = available && !hasUnsavedChanges;
	const blockerCopy: Record<HostingBlocker, { reason: string; fix?: string }> =
		{
			no_route: {
				reason: t(
					"hostingBlockerNoRoute",
					"The event has no route. Set a route path under Identity; hosted links address routes, not events.",
				),
			},
			inactive: {
				reason: t("hostingBlockerInactive", "The event is inactive."),
				fix: t("hostingFixActivate", "Activate"),
			},
			local_execution: {
				reason: t(
					"hostingBlockerLocal",
					"Execution is Local. Hosted links only run on the server.",
				),
				fix: t("hostingFixRemote", "Switch to Remote"),
			},
			internal_exposure: {
				reason: t(
					"hostingBlockerInternal",
					"Exposure is Internal, so there is no public endpoint.",
				),
				fix: t("hostingFixPublic", "Make public"),
			},
		};
	const copyUrl = async (value: string) => {
		try {
			await navigator.clipboard.writeText(value);
			toast.success(t("hostingLinkCopied", "Link copied"));
		} catch {
			toast.error(t("hostingLinkCopyFailed", "Could not copy the link"));
		}
	};

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
							"hostingEnableRouteDescription",
							"Publish this route of the app's hosted link. Visitors must sign in unless you separately allow anonymous access.",
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
			{hosting.enabled && blockers.length > 0 && (
				<Alert>
					<AlertTriangle className="size-4" />
					<AlertTitle>
						{t(
							"hostingBlockedTitleRoute",
							"This route returns 404 until you fix this:",
						)}
					</AlertTitle>
					<AlertDescription className="space-y-3">
						<ul className="w-full space-y-2">
							{blockers.map((blocker) => {
								const fix = HOSTING_BLOCKER_FIX[blocker];
								const label = blockerCopy[blocker].fix;
								return (
									<li
										key={blocker}
										className="flex flex-wrap items-center justify-between gap-2"
									>
										<span>{blockerCopy[blocker].reason}</span>
										{canWrite && onEventChange && fix && label && (
											<Button
												type="button"
												variant="outline"
												size="sm"
												onClick={() => onEventChange(fix)}
											>
												{label}
											</Button>
										)}
									</li>
								);
							})}
						</ul>
						<p>
							{t(
								"hostingBlockedSave",
								"Then save the event. The hosted link reads the saved settings.",
							)}
						</p>
					</AlertDescription>
				</Alert>
			)}
			{url && (
				<div className="space-y-2">
					<div className="flex items-center gap-2">
						<Label htmlFor="frontend-hosting-url">
							{t("hostingRouteLink", "Link to this route")}
						</Label>
						<Badge variant={live ? "secondary" : "outline"}>
							{live
								? t("hostingLinkLive", "Live")
								: t("hostingLinkNotLive", "Not live")}
						</Badge>
					</div>
					<div className="flex items-center gap-2">
						<Input
							id="frontend-hosting-url"
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
							disabled={!live}
						>
							<a
								href={live ? url : undefined}
								target="_blank"
								rel="noopener noreferrer"
								aria-label={t("hostingOpenPage", "Open hosted page")}
								aria-disabled={!live}
							>
								<ExternalLink className="size-4" />
							</a>
						</Button>
					</div>
					<p className="text-xs text-muted-foreground">
						{t(
							"hostingRouteLinkHint",
							"Every published route of this app shares the same link and differs only in its path. Navigation between pages keeps visitors on it, as long as the target route is published with the same sign-in setting.",
						)}
					</p>
				</div>
			)}
		</div>
	);
}
