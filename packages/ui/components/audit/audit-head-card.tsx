"use client";

import { useTranslation } from "@flow-like/locales";
import { Anchor, ChevronDown, Loader2 } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { apiErrorMessage } from "../../lib/api-error";
import type { IProfile } from "../../lib/schema/profile/profile";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { RelativeTime } from "../ui/relative-time";
import { Skeleton } from "../ui/skeleton";
import { Textarea } from "../ui/textarea";
import { AuditCopyButton, AuditHash } from "./audit-hash";
import { parseSavedHead } from "./saved-head";
import type { AuditHeadCheck, IAuditHead } from "./types";
import { useAuditHead, useAuditHeadCheck } from "./use-audit";

function HeadField({
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

function HeadCheckResult({ result }: Readonly<{ result: AuditHeadCheck }>) {
	const { t } = useTranslation("audit");
	if (result === "matches") {
		return <Badge>{t("headMatches", "Matches the current timeline")}</Badge>;
	}
	if (result === "archived") {
		return (
			<p className="text-xs text-muted-foreground">
				<Badge variant="secondary" className="mr-2">
					{t("headArchived", "Archived")}
				</Badge>
				{t(
					"headArchivedHint",
					"That epoch left the database. Check it against the monthly archive.",
				)}
			</p>
		);
	}
	return (
		<p className="text-xs text-destructive">
			<Badge variant="destructive" className="mr-2">
				{t("headDiffers", "Does not match")}
			</Badge>
			{t(
				"headDiffersHint",
				"The timeline was truncated or rewritten after this head was saved.",
			)}
		</p>
	);
}

function HeadCheckForm({
	profile,
}: Readonly<{ profile: IProfile | undefined }>) {
	const { t } = useTranslation("audit");
	const [text, setText] = useState("");
	const check = useAuditHeadCheck(profile);
	const request = useMemo(() => parseSavedHead(text), [text]);
	const invalid = text.trim().length > 0 && !request;

	return (
		<div className="space-y-2">
			<Textarea
				value={text}
				onChange={(event) => {
					setText(event.target.value);
					check.reset();
				}}
				placeholder={t("headCheckPlaceholder", "Paste a saved head as JSON")}
				aria-label={t("headCheckLabel", "Saved head")}
				className="min-h-24 font-mono text-xs"
			/>
			{invalid && (
				<p className="text-xs text-destructive">
					{t(
						"headCheckInvalid",
						"Expected JSON with an epoch sequence and a 64-character hex hash, and a well-formed seal if the head has one.",
					)}
				</p>
			)}
			{request?.seal_seq != null && (
				<p className="break-all text-xs text-muted-foreground">
					{t("headCheckWithSeal", "Also checks seal #{{seq}} of {{chain}}.", {
						seq: request.seal_seq,
						chain: request.chain_id,
					})}
				</p>
			)}
			<div className="flex flex-wrap items-center gap-2">
				<Button
					size="sm"
					variant="outline"
					disabled={!request || check.isPending}
					onClick={() => request && check.mutate(request)}
				>
					{check.isPending && (
						<Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
					)}
					{t("headCheck", "Check against the timeline")}
				</Button>
				{check.data && <HeadCheckResult result={check.data.result} />}
				{check.isError && (
					<p role="alert" className="text-xs text-destructive">
						{apiErrorMessage(
							check.error,
							t("headCheckFailed", "Could not check this head."),
						)}
					</p>
				)}
			</div>
		</div>
	);
}

function HeadDetails({ head }: Readonly<{ head: IAuditHead }>) {
	const { t } = useTranslation("audit");
	const json = useMemo(() => JSON.stringify(head, null, 2), [head]);
	if (!head.seal) {
		return (
			<p className="text-sm text-muted-foreground">
				{t(
					"headNone",
					"No anchored seal yet. The audit worker anchors new seals within minutes when an audit signing key is configured.",
				)}
			</p>
		);
	}
	return (
		<div className="space-y-3">
			<div className="space-y-1.5 rounded-md border bg-muted/30 p-3">
				<HeadField label={t("headSeal", "Seal")}>
					<span className="font-mono">#{head.seal.seq}</span>
				</HeadField>
				<HeadField label={t("headSealedAt", "Sealed")}>
					<RelativeTime value={head.seal.sealed_at_ms} />
				</HeadField>
				<HeadField label={t("headRecords", "Records in seal")}>
					<span className="font-mono">{head.seal.record_count}</span>
				</HeadField>
				<HeadField label={t("headSealHash", "Seal hash")}>
					<AuditHash value={head.seal.hash} />
				</HeadField>
			</div>
			{head.epoch && (
				<div className="space-y-1.5 rounded-md border bg-muted/30 p-3">
					<HeadField label={t("headEpoch", "Epoch")}>
						<span className="font-mono">#{head.epoch.seq}</span>
					</HeadField>
					<HeadField label={t("headEpochCreated", "Signed")}>
						<RelativeTime value={head.epoch.created_at_ms} />
					</HeadField>
					<HeadField label={t("headKid", "Key id")}>
						<span className="font-mono">{head.epoch.kid}</span>
					</HeadField>
					<HeadField label={t("headEpochHash", "Epoch hash")}>
						<AuditHash value={head.epoch.hash} />
					</HeadField>
				</div>
			)}
			<AuditCopyButton value={json} label={t("copyHead", "Copy head JSON")} />
		</div>
	);
}

export interface AuditHeadCardProps {
	profile: IProfile | undefined;
	chainId: string;
}

export function AuditHeadCard({
	profile,
	chainId,
}: Readonly<AuditHeadCardProps>) {
	const { t } = useTranslation("audit");
	const head = useAuditHead(profile, chainId);

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<Anchor className="h-4 w-4 text-primary" />
					{t("headTitle", "Chain head")}
				</CardTitle>
				<CardDescription>
					{t(
						"headDescription",
						"The newest anchored seal and the signed epoch that covers it. Keep a copy outside the platform to detect a rewritten history later.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{head.isLoading ? (
					<Skeleton className="h-32 w-full" />
				) : head.isError ? (
					<p role="alert" className="text-sm text-destructive">
						{apiErrorMessage(
							head.error,
							t("headFailed", "Could not load the chain head."),
						)}
					</p>
				) : head.data ? (
					<HeadDetails head={head.data} />
				) : null}
				<Collapsible>
					<CollapsibleTrigger asChild>
						<Button
							variant="ghost"
							size="sm"
							className="group -ml-2 gap-1 text-xs text-muted-foreground"
						>
							<ChevronDown className="h-3.5 w-3.5 transition-transform group-data-[state=open]:rotate-180" />
							{t("headCheckToggle", "Check a saved head")}
						</Button>
					</CollapsibleTrigger>
					<CollapsibleContent className="pt-2">
						<HeadCheckForm profile={profile} />
					</CollapsibleContent>
				</Collapsible>
			</CardContent>
		</Card>
	);
}
