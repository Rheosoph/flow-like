"use client";

import {
	AlertTriangleIcon,
	CheckCircle2Icon,
	CheckIcon,
	GitForkIcon,
	HardDriveIcon,
	KeyRoundIcon,
	Loader2Icon,
	MinusIcon,
	ShieldAlertIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	type IBeginOfflineForkResponse,
	type IForkPreviewResponse,
	type IForkPreviewTarget,
	type IOnlineForkBody,
	type IOnlineForkResponse,
	isTokenReplaceable,
	siteEventId,
} from "../../../lib/schema/app/fork";
import { PatSelectorDialog } from "../../pat-selector-dialog";
import { Alert, AlertDescription, AlertTitle } from "../../ui/alert";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";

/**
 * The two response shapes the server returns from `/fork` (online → online)
 * and `/fork/offline/begin` (online → offline). Both share the same
 * request body shape (`IOnlineForkBody`), so the dialog itself stays
 * variant-agnostic and the host picks which `beginFork` to wire up.
 */
export type IBeginForkResponse =
	| IBeginOfflineForkResponse
	| IOnlineForkResponse;

export interface ForkTargetOption {
	value: IForkPreviewTarget;
	label: string;
	description: string;
}

export const DEFAULT_FORK_TARGET_OPTIONS: Record<
	IForkPreviewTarget,
	ForkTargetOption
> = {
	online: {
		value: "online",
		label: "Online account",
		description: "Create a private cloud copy on your account.",
	},
	offline: {
		value: "offline",
		label: "This device",
		description: "Download a local offline copy into the desktop app.",
	},
};

export function normalizeForkTargetOptions(
	target: IForkPreviewTarget,
	targets?: readonly IForkPreviewTarget[],
): ForkTargetOption[] {
	const values = targets?.length ? targets : [target];
	const deduped = Array.from(new Set(values));
	if (!deduped.includes(target)) deduped.unshift(target);
	return deduped.map((value) => DEFAULT_FORK_TARGET_OPTIONS[value]);
}

function isOfflineForkResponse(
	res: IBeginForkResponse,
): res is IBeginOfflineForkResponse {
	return "meta_blobs" in res;
}

export interface ForkAppDialogProps {
	appId: string;
	appName: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	target: IForkPreviewTarget;
	targetOptions?: ForkTargetOption[];
	onTargetChange?: (target: IForkPreviewTarget) => void;
	loadPreview: () => Promise<IForkPreviewResponse>;
	beginFork: (body: IOnlineForkBody) => Promise<IBeginForkResponse>;
	/**
	 * Called once `beginFork` resolves successfully. For offline forks
	 * the host (desktop) typically pulls the signed-URL bundle into the
	 * local store; for online forks the host typically navigates to the
	 * new app. This dialog only coordinates preview + token collection
	 * + initial begin call.
	 */
	onForkStarted?: (response: IBeginForkResponse) => void;
}

type DialogStage = "loading" | "preview" | "submitting" | "done" | "error";

export function ForkAppDialog({
	appId,
	appName,
	open,
	onOpenChange,
	target,
	targetOptions,
	onTargetChange,
	loadPreview,
	beginFork,
	onForkStarted,
}: Readonly<ForkAppDialogProps>) {
	const [stage, setStage] = useState<DialogStage>("loading");
	const [preview, setPreview] = useState<IForkPreviewResponse | null>(null);
	const [token, setToken] = useState("");
	const [showPatSelector, setShowPatSelector] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [response, setResponse] = useState<IBeginForkResponse | null>(null);
	const options =
		targetOptions && targetOptions.length > 0
			? targetOptions
			: [DEFAULT_FORK_TARGET_OPTIONS[target]];
	const selectedTargetOption =
		options.find((option) => option.value === target) ??
		DEFAULT_FORK_TARGET_OPTIONS[target];

	useEffect(() => {
		if (!open) {
			// Reset whenever the dialog closes so the next open is fresh.
			setStage("loading");
			setPreview(null);
			setToken("");
			setShowPatSelector(false);
			setError(null);
			setResponse(null);
			return;
		}
		let cancelled = false;
		setStage("loading");
		loadPreview()
			.then((p) => {
				if (cancelled) return;
				setPreview(p);
				setStage("preview");
			})
			.catch((err: unknown) => {
				if (cancelled) return;
				setError(
					err instanceof Error ? err.message : "Couldn't load fork preview",
				);
				setStage("error");
			});
		return () => {
			cancelled = true;
		};
	}, [open, loadPreview, target]);

	const replaceableSites = useMemo(() => {
		if (!preview) return [];
		return preview.remote_token_sites.filter(isTokenReplaceable);
	}, [preview]);

	const reauthSites = useMemo(() => {
		if (!preview) return [];
		return preview.remote_token_sites.filter((s) => !isTokenReplaceable(s));
	}, [preview]);

	const tokenRequired = target === "online" && replaceableSites.length > 0;
	const canSubmit =
		!!preview?.user_can_fork &&
		!!preview?.within_limits &&
		(!tokenRequired || token.trim().length > 0);

	const handleTargetChange = useCallback(
		(value: string) => {
			if (value !== "online" && value !== "offline") return;
			if (value === target || stage === "submitting") return;
			setPreview(null);
			setResponse(null);
			setError(null);
			setToken("");
			setStage("loading");
			onTargetChange?.(value);
		},
		[target, stage, onTargetChange],
	);

	const handleFork = useCallback(async () => {
		if (!preview) return;
		setStage("submitting");
		setError(null);
		try {
			const body: IOnlineForkBody = {
				remote_event_token: tokenRequired ? token.trim() : undefined,
			};
			const res = await beginFork(body);
			setResponse(res);
			setStage("done");
			onForkStarted?.(res);
			if (isOfflineForkResponse(res)) {
				toast.success(
					`Fork ready — ${res.meta_blobs.length} meta artifact${res.meta_blobs.length === 1 ? "" : "s"} inline, content pulled via signed prefix`,
				);
			} else {
				toast.success("Fork created on your account");
			}
		} catch (err) {
			const message =
				err instanceof Error ? err.message : "Failed to start fork";
			setError(message);
			setStage("preview");
			toast.error(message);
		}
	}, [preview, tokenRequired, token, beginFork, onForkStarted]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<GitForkIcon className="w-4 h-4" />
						Fork {appName}
					</DialogTitle>
					<DialogDescription>
						A fresh, secret-stripped copy will be created{" "}
						{target === "offline" ? "on this device" : "on your account"}.
						Variables marked `secret` are cleared and OAuth bindings will need
						to be re-authenticated on the new app.
					</DialogDescription>
				</DialogHeader>

				{options.length > 1 && stage !== "done" && (
					<div className="space-y-2">
						<Label className="text-sm font-medium">Fork destination</Label>
						<Select
							value={target}
							onValueChange={handleTargetChange}
							disabled={stage === "submitting"}
						>
							<SelectTrigger className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{options.map((option) => (
									<SelectItem key={option.value} value={option.value}>
										{option.label}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<p className="text-xs text-muted-foreground">
							{selectedTargetOption.description}
						</p>
					</div>
				)}

				{stage === "loading" && (
					<div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
						<Loader2Icon className="w-4 h-4 animate-spin" />
						Loading preview…
					</div>
				)}

				{stage === "error" && (
					<Alert variant="destructive">
						<AlertTriangleIcon className="w-4 h-4" />
						<AlertTitle>Couldn't load preview</AlertTitle>
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				)}

				{(stage === "preview" || stage === "submitting") && preview && (
					<div className="space-y-4">
						<ForkPreviewSummary preview={preview} appId={appId} />

						<ForkContentsSummary preview={preview} />

						{!preview.allow_forking && (
							<Alert variant="destructive">
								<ShieldAlertIcon className="w-4 h-4" />
								<AlertTitle>Forking is disabled on this app</AlertTitle>
								<AlertDescription>
									The owner has not opted in to forking yet. Ask them to enable
									the toggle in app settings.
								</AlertDescription>
							</Alert>
						)}
						{preview.allow_forking && !preview.user_can_fork && (
							<Alert variant="destructive">
								<ShieldAlertIcon className="w-4 h-4" />
								<AlertTitle>You can't fork this app</AlertTitle>
								<AlertDescription>
									{preview.disallow_reason ||
										"You don't have the required read permissions on the source app."}
								</AlertDescription>
							</Alert>
						)}
						{!preview.within_limits && (
							<Alert variant="destructive">
								<AlertTriangleIcon className="w-4 h-4" />
								<AlertTitle>Source exceeds the fork size cap</AlertTitle>
								<AlertDescription>
									This deployment caps forks at{" "}
									{formatBytes(preview.max_size_bytes)} /{" "}
									{preview.max_file_count.toLocaleString()} files.
								</AlertDescription>
							</Alert>
						)}

						{tokenRequired && (
							<div className="space-y-2">
								<Label className="text-sm font-medium">
									Remote-event token
								</Label>
								{token ? (
									<div className="flex items-center gap-2 rounded-md border p-2">
										<KeyRoundIcon className="w-4 h-4 text-green-600 dark:text-green-400" />
										<span className="text-sm flex-1">Token selected</span>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => setShowPatSelector(true)}
											disabled={stage === "submitting"}
										>
											Change
										</Button>
									</div>
								) : (
									<Button
										variant="outline"
										onClick={() => setShowPatSelector(true)}
										disabled={stage === "submitting"}
										className="w-full justify-start gap-2"
									>
										<KeyRoundIcon className="w-4 h-4" />
										Select or create token
									</Button>
								)}
								<p className="text-xs text-muted-foreground">
									Will be reused at {replaceableSites.length}{" "}
									{replaceableSites.length === 1 ? "site" : "sites"} (HTTP
									auth_token, PAT). OAuth bindings can't be substituted with a
									token; you'll need to re-auth on the fork.
								</p>
							</div>
						)}

						{reauthSites.length > 0 && (
							<Alert>
								<ShieldAlertIcon className="w-4 h-4" />
								<AlertTitle>OAuth re-auth required</AlertTitle>
								<AlertDescription>
									{reauthSites.length}{" "}
									{reauthSites.length === 1 ? "event" : "events"} use OAuth and
									will be cleared. After the fork is created, re-link the
									providers under those events.
								</AlertDescription>
							</Alert>
						)}
					</div>
				)}

				{stage === "done" && response && (
					<Alert>
						<CheckCircle2Icon className="w-4 h-4" />
						<AlertTitle>Fork created</AlertTitle>
						<AlertDescription className="space-y-1">
							<p>
								New app id:{" "}
								<code className="text-xs">{response.new_app_id}</code>
							</p>
							{isOfflineForkResponse(response) && (
								<p>
									{response.meta_blobs.length}{" "}
									{response.meta_blobs.length === 1 ? "artifact" : "artifacts"}{" "}
									shipped inline, content pulled from{" "}
									<code className="text-xs">
										{response.source_content_prefix}
									</code>
									. Credentials expire at{" "}
									<code className="text-xs">
										{response.expires_at ?? "soon"}
									</code>
									.
								</p>
							)}
							{response.report.skipped.length > 0 && (
								<p>
									{response.report.skipped.length} item(s) were skipped — see
									the destination app for details.
								</p>
							)}
						</AlertDescription>
					</Alert>
				)}

				<DialogFooter>
					{stage === "done" ? (
						<Button onClick={() => onOpenChange(false)}>Close</Button>
					) : (
						<>
							<Button
								variant="outline"
								onClick={() => onOpenChange(false)}
								disabled={stage === "submitting"}
							>
								Cancel
							</Button>
							<Button
								onClick={handleFork}
								disabled={!canSubmit || stage !== "preview"}
							>
								{stage === "submitting" ? (
									<>
										<Loader2Icon className="w-4 h-4 animate-spin mr-2" />
										Forking…
									</>
								) : (
									`Fork to ${
										target === "offline" ? "this device" : "my account"
									}`
								)}
							</Button>
						</>
					)}
				</DialogFooter>
			</DialogContent>
			<PatSelectorDialog
				open={showPatSelector}
				onOpenChange={setShowPatSelector}
				onPatSelected={(pat) => {
					setToken(pat);
					setShowPatSelector(false);
				}}
				title="Select or Create Fork Token"
				description="Choose an existing token or create a new one. It will replace HTTP auth tokens and PATs at remote-event sites in your fork."
			/>
		</Dialog>
	);
}

function ForkPreviewSummary({
	preview,
	appId,
}: Readonly<{ preview: IForkPreviewResponse; appId: string }>) {
	return (
		<div className="grid grid-cols-2 gap-3 rounded-md border p-4 text-sm">
			<div className="flex items-center gap-2">
				<HardDriveIcon className="w-4 h-4 text-muted-foreground" />
				<span className="text-muted-foreground">Size</span>
			</div>
			<div className="text-right font-medium">
				{formatBytes(preview.selected_size_bytes)}
				{preview.selected_size_bytes !== preview.total_size_bytes && (
					<div className="text-xs font-normal text-muted-foreground">
						of {formatBytes(preview.total_size_bytes)} total
					</div>
				)}
			</div>
			<div className="text-muted-foreground">Files</div>
			<div className="text-right font-medium">
				{preview.selected_object_count.toLocaleString()}
			</div>
			<div className="text-muted-foreground">Cap</div>
			<div className="text-right text-xs text-muted-foreground">
				{formatBytes(preview.max_size_bytes)} /{" "}
				{preview.max_file_count.toLocaleString()} files
			</div>
			{preview.remote_token_sites.length > 0 && (
				<>
					<div className="text-muted-foreground">Token sites</div>
					<div className="text-right">
						{preview.remote_token_sites.length}
						<div className="text-xs text-muted-foreground">
							{preview.remote_token_sites
								.slice(0, 3)
								.map((s) => siteEventId(s))
								.join(", ")}
							{preview.remote_token_sites.length > 3 && "…"}
						</div>
					</div>
				</>
			)}
			<div className="text-muted-foreground col-span-2 text-xs pt-2 border-t">
				Source: <code className="text-xs">{appId}</code>
			</div>
		</div>
	);
}

/**
 * What the source app's owner allows a fork to contain. Read-only — the
 * forker doesn't choose; this just sets expectations before they commit.
 */
function ForkContentsSummary({
	preview,
}: Readonly<{ preview: IForkPreviewResponse }>) {
	const policy = preview.fork_policy;
	const sizes = preview.size_breakdown;
	const databaseSize =
		policy.databases === "with_data" ? sizeHint(sizes.databases) : undefined;
	const databaseDetail =
		policy.databases === "with_data"
			? ["Tables and data", databaseSize].filter(Boolean).join(" · ")
			: "Tables only, no data";

	const rows: readonly {
		label: string;
		included: boolean;
		detail?: string;
	}[] = [
		{ label: "Flows", included: policy.flows, detail: sizeHint(sizes.flows) },
		{ label: "Files", included: policy.files, detail: sizeHint(sizes.files) },
		{
			label: "Databases",
			included: policy.databases !== "none",
			detail: databaseDetail,
		},
		{
			label: "Widgets",
			included: policy.widgets,
			detail: sizeHint(sizes.widgets),
		},
		{
			label: "Templates",
			included: policy.templates,
			detail: sizeHint(sizes.templates),
		},
		{ label: "Roles", included: policy.roles },
	];

	return (
		<div className="rounded-md border p-4 space-y-2">
			<p className="text-xs text-muted-foreground">
				The app owner decides what a fork includes.
			</p>
			<ul className="space-y-1.5">
				{rows.map((row) => (
					<li
						key={row.label}
						className="flex items-center justify-between gap-3 text-sm"
					>
						<span className="flex items-center gap-2">
							{row.included ? (
								<CheckIcon className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-500" />
							) : (
								<MinusIcon className="w-3.5 h-3.5 text-muted-foreground" />
							)}
							<span
								className={
									row.included
										? undefined
										: "text-muted-foreground line-through"
								}
							>
								{row.label}
							</span>
						</span>
						<span className="text-xs text-muted-foreground">
							{row.included ? row.detail : "Not included"}
						</span>
					</li>
				))}
			</ul>
			{!policy.flows && (
				<p className="text-xs text-muted-foreground border-t pt-2">
					Without flows this copy has no runnable logic — its events and pages
					come with the boards, so they aren't included either.
				</p>
			)}
		</div>
	);
}

function sizeHint(size: { bytes: number }): string | undefined {
	return size.bytes > 0 ? formatBytes(size.bytes) : undefined;
}

function formatBytes(bytes: number): string {
	if (bytes === 0) return "0 B";
	const units = ["B", "KB", "MB", "GB", "TB"];
	const i = Math.min(
		units.length - 1,
		Math.floor(Math.log(bytes) / Math.log(1024)),
	);
	const value = bytes / 1024 ** i;
	return `${value.toFixed(value >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}
