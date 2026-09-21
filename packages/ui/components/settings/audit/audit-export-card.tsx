"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Download,
	KeyRound,
	Loader2,
	RotateCw,
	Trash2,
	TriangleAlert,
	Webhook,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../lib/api-error";
import { getApiUrl } from "../../../lib/api-url";
import type { IProfile } from "../../../lib/schema/profile/profile";
import { AuditCopyButton } from "../../audit/audit-hash";
import type { AuditExportClass, IAuditWebhook } from "../../audit/types";
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
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button, buttonVariants } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { RelativeTime } from "../../ui/relative-time";
import { Separator } from "../../ui/separator";
import { Skeleton } from "../../ui/skeleton";
import { Switch } from "../../ui/switch";
import { useAuditWebhook } from "./use-audit-webhook";

const EXPORT_PAGE_LIMIT = 100;

function isAcceptableUrl(value: string): boolean {
	try {
		const url = new URL(value);
		return url.protocol === "https:" || url.protocol === "http:";
	} catch {
		return false;
	}
}

function SecretReveal({
	secret,
	onDismiss,
}: Readonly<{ secret: string; onDismiss: () => void }>) {
	const { t } = useTranslation("audit");
	return (
		<Alert>
			<KeyRound className="h-4 w-4" />
			<AlertTitle>
				{t(
					"secretOnce",
					"Copy this signing secret now. It is shown only once.",
				)}
			</AlertTitle>
			<AlertDescription className="space-y-2">
				<code className="block break-all rounded bg-muted px-2 py-1.5 font-mono text-xs text-foreground">
					{secret}
				</code>
				<div className="flex gap-2">
					<AuditCopyButton
						value={secret}
						label={t("copySecret", "Copy secret")}
					/>
					<Button size="sm" variant="ghost" onClick={onDismiss}>
						{t("secretStored", "I stored it")}
					</Button>
				</div>
			</AlertDescription>
		</Alert>
	);
}

function WebhookForm({
	webhook,
	saving,
	onSave,
}: Readonly<{
	webhook: IAuditWebhook | null;
	saving: boolean;
	onSave: (url: string, active: boolean) => void;
}>) {
	const { t } = useTranslation("audit");
	const [url, setUrl] = useState(webhook?.url ?? "");
	const [active, setActive] = useState(webhook?.active ?? true);
	const trimmed = url.trim();
	const valid = isAcceptableUrl(trimmed);
	const unchanged =
		!!webhook && webhook.url === trimmed && webhook.active === active;

	return (
		<form
			className="space-y-3"
			onSubmit={(event) => {
				event.preventDefault();
				if (valid && !unchanged) onSave(trimmed, active);
			}}
		>
			<div className="space-y-1.5">
				<Label htmlFor="audit-webhook-url">
					{t("webhookUrl", "Webhook URL")}
				</Label>
				<Input
					id="audit-webhook-url"
					type="url"
					inputMode="url"
					value={url}
					onChange={(event) => setUrl(event.target.value)}
					placeholder="https://siem.example.com/flow-like/audit"
					className="font-mono text-sm"
				/>
				{trimmed.startsWith("http://") && (
					<p className="text-xs text-muted-foreground">
						{t(
							"webhookHttpHint",
							"Plain http is only accepted by deployments that allow it for local development.",
						)}
					</p>
				)}
			</div>
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div className="flex items-center gap-2">
					<Switch
						id="audit-webhook-active"
						checked={active}
						onCheckedChange={setActive}
					/>
					<Label htmlFor="audit-webhook-active" className="text-sm">
						{t("webhookActive", "Deliver records")}
					</Label>
				</div>
				<Button
					type="submit"
					size="sm"
					disabled={!valid || unchanged || saving}
				>
					{saving && <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />}
					{webhook
						? t("webhookSave", "Save changes")
						: t("webhookCreate", "Create webhook")}
				</Button>
			</div>
		</form>
	);
}

function StatusRow({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="flex items-center justify-between gap-3 text-xs">
			<span className="text-muted-foreground">{label}</span>
			<span className="min-w-0 truncate text-right">{children}</span>
		</div>
	);
}

function WebhookStatus({ webhook }: Readonly<{ webhook: IAuditWebhook }>) {
	const { t } = useTranslation("audit");
	return (
		<div className="space-y-2 rounded-md border bg-muted/30 p-3">
			<StatusRow label={t("webhookStatus", "Status")}>
				{webhook.active ? (
					<Badge>{t("webhookDelivering", "Delivering")}</Badge>
				) : (
					<Badge variant="outline">{t("webhookPaused", "Paused")}</Badge>
				)}
			</StatusRow>
			<StatusRow label={t("webhookLastDelivery", "Last delivery")}>
				{webhook.last_delivered_at_ms != null ? (
					<RelativeTime value={webhook.last_delivered_at_ms} />
				) : (
					t("webhookNever", "Never")
				)}
			</StatusRow>
			<StatusRow label={t("webhookFailures", "Consecutive failures")}>
				<span
					className={
						webhook.failures > 0 ? "font-mono text-destructive" : "font-mono"
					}
				>
					{webhook.failures}
				</span>
			</StatusRow>
			{webhook.next_attempt_at_ms != null && webhook.failures > 0 && (
				<StatusRow label={t("webhookNextAttempt", "Next attempt")}>
					<RelativeTime value={webhook.next_attempt_at_ms} />
				</StatusRow>
			)}
			<StatusRow label={t("webhookCursors", "Delivered through seal")}>
				<span className="font-mono">
					{t(
						"webhookCursorValues",
						"evidence #{{evidence}} · activity #{{activity}}",
						{
							evidence: webhook.evidence_cursor,
							activity: webhook.activity_cursor,
						},
					)}
				</span>
			</StatusRow>
			<StatusRow label={t("webhookCreated", "Created")}>
				<RelativeTime value={webhook.created_at_ms} />
			</StatusRow>
			{webhook.last_error && (
				<Alert variant="destructive" className="mt-2">
					<TriangleAlert className="h-4 w-4" />
					<AlertTitle>{t("webhookLastError", "Last error")}</AlertTitle>
					<AlertDescription className="font-mono text-xs">
						{webhook.last_error}
					</AlertDescription>
				</Alert>
			)}
			{!webhook.active && webhook.failures >= 50 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"webhookDisabledHint",
						"Delivery stopped after repeated failures. Fix the endpoint, then switch delivery back on.",
					)}
				</p>
			)}
		</div>
	);
}

function ConfirmButton({
	icon,
	label,
	title,
	description,
	confirmLabel,
	destructive = false,
	disabled,
	onConfirm,
}: Readonly<{
	icon: ReactNode;
	label: string;
	title: string;
	description: string;
	confirmLabel: string;
	destructive?: boolean;
	disabled?: boolean;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("audit");
	return (
		<AlertDialog>
			<AlertDialogTrigger asChild>
				<Button
					size="sm"
					variant="outline"
					className={destructive ? "gap-1.5 text-destructive" : "gap-1.5"}
					disabled={disabled}
				>
					{icon}
					{label}
				</Button>
			</AlertDialogTrigger>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>{title}</AlertDialogTitle>
					<AlertDialogDescription>{description}</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
					<AlertDialogAction
						onClick={onConfirm}
						className={
							destructive
								? buttonVariants({ variant: "destructive" })
								: undefined
						}
					>
						{confirmLabel}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function PullEndpoints({
	profile,
	appId,
}: Readonly<{ profile: IProfile | undefined; appId: string }>) {
	const { t } = useTranslation("audit");
	const urls = useMemo(
		() =>
			(["evidence", "activity"] as const).map(
				(exportClass: AuditExportClass) => {
					const params = new URLSearchParams({
						class: exportClass,
						after_seq: "0",
						limit: `${EXPORT_PAGE_LIMIT}`,
					});
					return {
						exportClass,
						url: getApiUrl(
							profile,
							`apps/${encodeURIComponent(appId)}/audit/export?${params.toString()}`,
						),
					};
				},
			),
		[profile, appId],
	);

	return (
		<div className="space-y-3">
			<div className="flex items-center gap-2 text-sm font-medium">
				<Download className="h-4 w-4 text-muted-foreground" />
				{t("pullTitle", "Pull instead")}
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"pullDescription",
					"Fetch anchored seals and their records as NDJSON with an owner's access token. Pass the X-FlowLike-Audit-Next-Seq response header as after_seq to read the next page.",
				)}
			</p>
			{urls.map(({ exportClass, url }) => (
				<div key={exportClass} className="space-y-1">
					<div className="text-xs text-muted-foreground">
						{exportClass === "evidence"
							? t("classEvidence", "Evidence")
							: t("classActivity", "Activity")}
					</div>
					<div className="flex items-center gap-2">
						<code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1.5 font-mono text-xs">
							{url}
						</code>
						<AuditCopyButton value={url} label={t("copy", "Copy")} />
					</div>
				</div>
			))}
		</div>
	);
}

export interface AuditExportCardProps {
	profile: IProfile | undefined;
	appId: string;
}

export function AuditExportCard({
	profile,
	appId,
}: Readonly<AuditExportCardProps>) {
	const { t } = useTranslation("audit");
	const { webhook, save, rotate, remove } = useAuditWebhook(profile, appId);
	const [secret, setSecret] = useState<string | null>(null);
	const current = webhook.data ?? null;
	const busy = save.isPending || rotate.isPending || remove.isPending;

	const handleSave = (url: string, active: boolean) =>
		save.mutate(
			{ url, active },
			{
				onSuccess: (saved) => {
					if (saved.secret) setSecret(saved.secret);
					toast.success(t("webhookSaved", "Webhook saved"));
				},
				onError: (error) =>
					toast.error(
						apiErrorMessage(
							error,
							t("webhookSaveFailed", "Could not save the webhook."),
						),
					),
			},
		);

	const handleRotate = () =>
		rotate.mutate(undefined, {
			onSuccess: ({ secret: next }) => {
				setSecret(next);
				toast.success(t("secretRotated", "Signing secret rotated"));
			},
			onError: (error) =>
				toast.error(
					apiErrorMessage(
						error,
						t("secretRotateFailed", "Could not rotate the secret."),
					),
				),
		});

	const handleRemove = () =>
		remove.mutate(undefined, {
			onSuccess: () => {
				setSecret(null);
				toast.success(t("webhookRemoved", "Webhook removed"));
			},
			onError: (error) =>
				toast.error(
					apiErrorMessage(
						error,
						t("webhookRemoveFailed", "Could not remove the webhook."),
					),
				),
		});

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<Webhook className="h-4 w-4 text-primary" />
					{t("exportTitle", "Export")}
				</CardTitle>
				<CardDescription>
					{t(
						"exportDescription",
						"Keep this app's audit trail longer than the platform does: every anchored seal and its records are delivered to your SIEM or storage within minutes.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-5">
				{secret && (
					<SecretReveal secret={secret} onDismiss={() => setSecret(null)} />
				)}
				{webhook.isLoading ? (
					<Skeleton className="h-28 w-full" />
				) : webhook.isError ? (
					<p role="alert" className="text-sm text-destructive">
						{apiErrorMessage(
							webhook.error,
							t("webhookLoadFailed", "Could not load the export settings."),
						)}
					</p>
				) : (
					<>
						<WebhookForm
							key={current ? `${current.url}:${current.active}` : "new"}
							webhook={current}
							saving={save.isPending}
							onSave={handleSave}
						/>
						{current && (
							<>
								<WebhookStatus webhook={current} />
								<div className="flex flex-wrap gap-2">
									<ConfirmButton
										icon={<RotateCw className="h-3.5 w-3.5" />}
										label={t("rotateSecret", "Rotate secret")}
										title={t("rotateTitle", "Rotate the signing secret?")}
										description={t(
											"rotateDescription",
											"Deliveries are signed with the new secret right away. Update your receiver before the next delivery.",
										)}
										confirmLabel={t("rotateConfirm", "Rotate")}
										disabled={busy}
										onConfirm={handleRotate}
									/>
									<ConfirmButton
										icon={<Trash2 className="h-3.5 w-3.5" />}
										label={t("removeWebhook", "Remove webhook")}
										title={t("removeTitle", "Remove the export webhook?")}
										description={t(
											"removeDescription",
											"Delivery stops and the delivery position is lost. A new webhook starts again from the oldest record still in the platform.",
										)}
										confirmLabel={t("removeConfirm", "Remove")}
										destructive
										disabled={busy}
										onConfirm={handleRemove}
									/>
								</div>
								<p className="text-xs text-muted-foreground">
									{t(
										"signatureHint",
										'Each delivery carries X-FlowLike-Audit-Timestamp and X-FlowLike-Audit-Signature: v1=<hex HMAC-SHA256 of "<timestamp>.<body>" with your secret>.',
									)}
								</p>
							</>
						)}
					</>
				)}
				<Separator />
				<PullEndpoints profile={profile} appId={appId} />
			</CardContent>
		</Card>
	);
}
