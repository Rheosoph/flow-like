"use client";

import { useTranslation } from "@flow-like/locales";
import { Clock3, ShieldCheck, Sparkles } from "lucide-react";
import { useId } from "react";
import {
	RETENTION_DAY_OPTIONS,
	RETENTION_KINDS,
	type RetentionKind,
	type StorageRetentionPolicy,
} from "../../../lib/browser-storage/retention";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { formatStorageDate } from "./format";
import { useStorageCopy } from "./use-storage-copy";

const FOREVER = "forever";

interface RetentionCardProps {
	readonly policy: StorageRetentionPolicy;
	readonly lastRunAt?: number;
	readonly busy: boolean;
	readonly onPolicyChange: (kind: RetentionKind, days: number | null) => void;
	readonly onRunNow: () => void;
}

export function RetentionCard({
	policy,
	lastRunAt,
	busy,
	onPolicyChange,
	onRunNow,
}: RetentionCardProps) {
	const { t } = useTranslation("settings");
	const copy = useStorageCopy();
	const id = useId();

	return (
		<Card className="overflow-hidden">
			<CardHeader>
				<div className="flex gap-3">
					<div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
						<Sparkles className="size-4.5" />
					</div>
					<div>
						<CardTitle className="text-base">
							{t("storageAutomaticCleanup", "Automatic cleanup")}
						</CardTitle>
						<CardDescription className="mt-1">
							{t(
								"storageAutomaticCleanupDescription",
								"Old history is removed each time Flow-Like opens in this browser.",
							)}
						</CardDescription>
					</div>
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="space-y-2.5">
					{RETENTION_KINDS.map((kind) => {
						const days = policy[kind];
						const options =
							days === null ||
							(RETENTION_DAY_OPTIONS as readonly number[]).includes(days)
								? RETENTION_DAY_OPTIONS
								: [...RETENTION_DAY_OPTIONS, days].sort((a, b) => a - b);
						return (
							<div
								key={kind}
								className="flex items-center justify-between gap-3"
							>
								<Label
									htmlFor={`${id}-${kind}`}
									className="min-w-0 text-sm font-normal"
								>
									{copy.retention[kind]}
								</Label>
								<Select
									value={days === null ? FOREVER : String(days)}
									disabled={busy}
									onValueChange={(value) =>
										onPolicyChange(
											kind,
											value === FOREVER ? null : Number(value),
										)
									}
								>
									<SelectTrigger
										id={`${id}-${kind}`}
										size="sm"
										className="w-36 shrink-0"
									>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={FOREVER}>
											{t("storageRetentionForever", "Keep forever")}
										</SelectItem>
										{options.map((option) => (
											<SelectItem key={option} value={String(option)}>
												{t("storageRetentionDays", {
													count: option,
													defaultValue_one: "{{count}} day",
													defaultValue_other: "{{count}} days",
												})}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
						);
					})}
				</div>
				<div className="flex flex-wrap items-center justify-between gap-2 border-t pt-4">
					<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
						<Clock3 className="size-3.5" />
						{lastRunAt
							? t("storageLastCleanup", "Last cleanup {{date}}", {
									date: formatStorageDate(lastRunAt),
								})
							: t("storageNoCleanupYet", "Not cleaned up yet")}
					</p>
					<Button
						variant="secondary"
						size="sm"
						disabled={busy}
						onClick={onRunNow}
					>
						{t("storageCleanUpNow", "Clean up now")}
					</Button>
				</div>
				<div className="flex gap-2.5 rounded-lg bg-muted/60 p-3 text-xs text-muted-foreground">
					<ShieldCheck className="mt-0.5 size-4 shrink-0 text-foreground" />
					<p className="leading-relaxed">
						{t(
							"storageCleanupGuarantee",
							"Pinned conversations, changes waiting to sync, and your apps and boards are never removed by automatic cleanup.",
						)}
					</p>
				</div>
			</CardContent>
		</Card>
	);
}
