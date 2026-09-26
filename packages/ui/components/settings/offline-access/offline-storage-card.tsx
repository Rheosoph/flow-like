"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useId, useState } from "react";
import { humanFileSize } from "../../../lib/utils";
import type {
	IOfflineLimits,
	IOfflineUsage,
	IOfflineWritesState,
} from "../../../state/backend-state/offline-writes-state";
import { Button } from "../../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Progress } from "../../ui/progress";
import {
	DAY_SECONDS,
	MEBIBYTE,
	mirrorLimitError,
} from "./offline-access-logic";

function percent(used: number, limit: number): number {
	return limit > 0 ? Math.min(100, (used / limit) * 100) : 0;
}

function UsageBar({
	label,
	used,
	limit,
	children,
}: Readonly<{
	label: string;
	used: number;
	limit: number;
	children?: React.ReactNode;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-1.5">
			<div className="flex flex-wrap items-baseline justify-between gap-2 text-sm">
				<span className="font-medium">{label}</span>
				<span className="text-muted-foreground">
					{t("settings:offlineAccess.usage", "{{used}} of {{limit}} used", {
						used: humanFileSize(used),
						limit: humanFileSize(limit),
					})}
				</span>
			</div>
			<Progress value={percent(used, limit)} />
			{children}
		</div>
	);
}

function LimitField({
	label,
	unit,
	value,
	min,
	max,
	disabled,
	onChange,
}: Readonly<{
	label: string;
	unit: string;
	value: string;
	min: number;
	max: number;
	disabled: boolean;
	onChange: (value: string) => void;
}>) {
	const id = useId();
	return (
		<div className="space-y-1.5">
			<Label htmlFor={id}>{label}</Label>
			<div className="flex items-center gap-2">
				<Input
					id={id}
					type="number"
					inputMode="numeric"
					min={min}
					max={max}
					step={1}
					required
					value={value}
					disabled={disabled}
					onChange={(event) => onChange(event.target.value)}
				/>
				<span className="shrink-0 text-sm text-muted-foreground">{unit}</span>
			</div>
		</div>
	);
}

export function OfflineStorageCard({
	appId,
	state,
	limits,
	usage,
	onChanged,
}: Readonly<{
	appId: string;
	state: IOfflineWritesState;
	limits: IOfflineLimits;
	usage: IOfflineUsage;
	onChanged: () => void;
}>) {
	const { t } = useTranslation("settings");
	const [queueMiB, setQueueMiB] = useState("");
	const [mirrorMiB, setMirrorMiB] = useState("");
	const [ageDays, setAgeDays] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();

	const { maxQueueBytes, maxMirrorBytes, maxAgeSeconds } = limits;
	useEffect(() => {
		setQueueMiB(String(Math.round(maxQueueBytes / MEBIBYTE)));
		setMirrorMiB(String(Math.round(maxMirrorBytes / MEBIBYTE)));
		setAgeDays(String(Math.max(1, Math.round(maxAgeSeconds / DAY_SECONDS))));
	}, [maxQueueBytes, maxMirrorBytes, maxAgeSeconds]);

	const mebibytes = t("settings:offlineAccess.unitMebibytes", "MiB");

	const save = async () => {
		const next: IOfflineLimits = {
			...limits,
			maxQueueBytes: Math.round(Number(queueMiB) * MEBIBYTE),
			maxMirrorBytes: Math.round(Number(mirrorMiB) * MEBIBYTE),
			maxAgeSeconds: Math.round(Number(ageDays) * DAY_SECONDS),
		};
		const tooLow = mirrorLimitError(
			t,
			next.maxMirrorBytes,
			usage.requiredBytes,
		);
		if (tooLow) {
			setError(tooLow);
			return;
		}
		setBusy(true);
		setError(undefined);
		try {
			await state.setLimits(appId, next);
			onChanged();
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setBusy(false);
		}
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle>
					{t("settings:offlineAccess.storageTitle", "Storage on this device")}
				</CardTitle>
			</CardHeader>
			<CardContent className="space-y-5">
				<UsageBar
					label={t("settings:offlineAccess.queueLimit", "Queued changes limit")}
					used={usage.queueBytes}
					limit={maxQueueBytes}
				/>
				<UsageBar
					label={t(
						"settings:offlineAccess.mirrorLimit",
						"Offline copies limit",
					)}
					used={usage.mirrorBytes}
					limit={maxMirrorBytes}
				>
					{usage.pinnedBytes > 0 && (
						<p className="text-xs text-muted-foreground">
							{t(
								"settings:offlineAccess.pinnedUsage",
								"{{used}} kept for tables that download everything or have queued changes",
								{ used: humanFileSize(usage.pinnedBytes) },
							)}
						</p>
					)}
				</UsageBar>
				{usage.maxDownloadBytesPerDay != null && (
					<div className="space-y-1.5">
						<p className="text-sm text-muted-foreground">
							{t(
								"settings:offlineAccess.downloadedToday",
								"Downloaded today: {{used}} of {{limit}}",
								{
									used: humanFileSize(usage.downloadedBytesToday),
									limit: humanFileSize(usage.maxDownloadBytesPerDay),
								},
							)}
						</p>
						<Progress
							value={percent(
								usage.downloadedBytesToday,
								usage.maxDownloadBytesPerDay,
							)}
						/>
					</div>
				)}

				<form
					className="grid gap-4 sm:grid-cols-3"
					onSubmit={(event) => {
						event.preventDefault();
						void save();
					}}
				>
					<LimitField
						label={t(
							"settings:offlineAccess.queueLimit",
							"Queued changes limit",
						)}
						unit={mebibytes}
						value={queueMiB}
						min={1}
						max={64 * 1024}
						disabled={busy}
						onChange={setQueueMiB}
					/>
					<LimitField
						label={t(
							"settings:offlineAccess.mirrorLimit",
							"Offline copies limit",
						)}
						unit={mebibytes}
						value={mirrorMiB}
						min={1}
						max={1024 * 1024}
						disabled={busy}
						onChange={setMirrorMiB}
					/>
					<LimitField
						label={t("settings:offlineAccess.maxAge", "Oldest change allowed")}
						unit={t("settings:offlineAccess.unitDays", "days")}
						value={ageDays}
						min={1}
						max={30}
						disabled={busy}
						onChange={setAgeDays}
					/>
					{error && (
						<p role="alert" className="text-sm text-destructive sm:col-span-3">
							{error}
						</p>
					)}
					<div className="sm:col-span-3">
						<Button type="submit" size="sm" disabled={busy}>
							{t("common:save", "Save")}
						</Button>
					</div>
				</form>
			</CardContent>
		</Card>
	);
}
