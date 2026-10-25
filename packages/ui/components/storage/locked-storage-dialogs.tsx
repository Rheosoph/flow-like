"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useId, useState } from "react";
import { humanFileSize } from "../../lib/utils";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Button,
	Checkbox,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Textarea,
} from "../ui";
import { LockedItemDetails } from "./locked-item-details";
import {
	formatLockedDate,
	nextRetentionDay,
	retentionFromDay,
} from "./locked-storage-entries";
import { UploadProgressPanel } from "./storage-upload-progress";
import type { LockedStorageController } from "./use-locked-storage";

type ControllerProps = Readonly<{ controller: LockedStorageController }>;

function validRetentionDays(value: string, min: number, max: number) {
	const days = Number(value);
	return (
		value.trim() !== "" && Number.isInteger(days) && days >= min && days <= max
	);
}

function LockedUploadDialog({ controller }: ControllerProps) {
	const { t } = useTranslation("common");
	const id = useId();
	const { status, uploadTarget } = controller;
	const open = !!uploadTarget && !!status;
	const versionOf = uploadTarget?.item;
	const objectMode = status?.enforcement === "object";
	const [files, setFiles] = useState<File[]>([]);
	const [days, setDays] = useState("");
	const [deleteAfter, setDeleteAfter] = useState(false);
	const [comment, setComment] = useState("");

	useEffect(() => {
		if (!open) return;
		setFiles([]);
		setComment("");
		setDeleteAfter(false);
		setDays(String(status?.default_retention_days ?? ""));
	}, [open, status?.default_retention_days]);

	if (!status) return null;
	const daysError =
		objectMode &&
		!validRetentionDays(
			days,
			status.min_retention_days,
			status.max_retention_days,
		);

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) controller.closeUpload();
			}}
		>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{versionOf
							? t(
									"lockedStorage.uploadVersionTitle",
									"New version of {{name}}",
									{
										name: versionOf.name,
									},
								)
							: t("lockedStorage.uploadLockedFiles", "Upload locked files")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"lockedStorage.uploadDescription",
							"Locked files cannot be changed, renamed or deleted until their retention ends. They count against your storage.",
						)}
					</DialogDescription>
				</DialogHeader>
				<form
					className="space-y-4"
					onSubmit={(event) => {
						event.preventDefault();
						if (files.length === 0 || daysError) return;
						controller.submitUpload(files, {
							retentionDays: objectMode ? Number(days) : undefined,
							deleteAfterRetention: controller.canPurge && deleteAfter,
							comment,
						});
					}}
				>
					<div className="space-y-2">
						<Label htmlFor={`${id}-files`}>
							{t("lockedStorage.files", "Files")}
						</Label>
						<Input
							id={`${id}-files`}
							type="file"
							multiple={!versionOf}
							onChange={(event) =>
								setFiles(Array.from(event.target.files ?? []))
							}
						/>
						{status.max_file_bytes > 0 && (
							<p className="text-xs text-muted-foreground">
								{t("lockedStorage.maxFileSize", "Up to {{size}} per file", {
									size: humanFileSize(status.max_file_bytes, true),
								})}
							</p>
						)}
					</div>
					<div className="space-y-2">
						<Label htmlFor={`${id}-days`}>
							{t("lockedStorage.retentionDays", "Retention in days")}
						</Label>
						{objectMode ? (
							<>
								<Input
									id={`${id}-days`}
									type="number"
									inputMode="numeric"
									min={status.min_retention_days}
									max={status.max_retention_days}
									value={days}
									aria-invalid={daysError}
									onChange={(event) => setDays(event.target.value)}
								/>
								<p
									className={`text-xs ${daysError ? "text-destructive" : "text-muted-foreground"}`}
								>
									{t(
										"lockedStorage.retentionRange",
										"Between {{min}} and {{max}} days. Space is freed only after retention ends.",
										{
											min: status.min_retention_days,
											max: status.max_retention_days,
										},
									)}
								</p>
							</>
						) : (
							<p id={`${id}-days`} className="text-sm text-muted-foreground">
								{t(
									"lockedStorage.bucketRetention",
									"Retention follows the storage rule: {{days}} days.",
									{ days: status.default_retention_days },
								)}
							</p>
						)}
					</div>
					{controller.canPurge && (
						<div className="flex items-start gap-3">
							<Checkbox
								id={`${id}-delete`}
								className="mt-0.5"
								checked={deleteAfter}
								onCheckedChange={(checked) => setDeleteAfter(checked === true)}
							/>
							<div className="space-y-1">
								<Label htmlFor={`${id}-delete`} className="cursor-pointer">
									{t(
										"lockedStorage.deleteAfterRetention",
										"Delete automatically when retention ends",
									)}
								</Label>
								<p className="text-xs text-muted-foreground">
									{t(
										"lockedStorage.deleteAfterRetentionHint",
										"Otherwise it stays stored and counts against your storage until someone deletes it. A legal hold postpones the deletion.",
									)}
								</p>
							</div>
						</div>
					)}
					<div className="space-y-2">
						<Label htmlFor={`${id}-comment`}>
							{t("lockedStorage.comment", "Comment")}
						</Label>
						<Textarea
							id={`${id}-comment`}
							value={comment}
							rows={2}
							onChange={(event) => setComment(event.target.value)}
						/>
					</div>
					<DialogFooter>
						<Button
							type="button"
							variant="outline"
							onClick={controller.closeUpload}
						>
							{t("lockedStorage.cancel", "Cancel")}
						</Button>
						<Button
							type="submit"
							disabled={
								files.length === 0 ||
								daysError ||
								controller.progress.isUploading
							}
						>
							{t("lockedStorage.lockAndUpload", "Lock and upload")}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function LockedConflictDialog({ controller }: ControllerProps) {
	const { t } = useTranslation("common");
	const { conflict } = controller;
	return (
		<AlertDialog
			open={!!conflict}
			onOpenChange={(next) => {
				if (!next) controller.resolveConflict(false);
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("lockedStorage.conflictTitle", "Already locked here")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"lockedStorage.conflictDescription",
							"{{names}} already exist as locked files in this folder. Upload them as new versions? Earlier versions stay stored and locked.",
							{ names: conflict?.names.join(", ") ?? "" },
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={() => controller.resolveConflict(false)}>
						{t("lockedStorage.cancelUpload", "Cancel upload")}
					</AlertDialogCancel>
					<AlertDialogAction onClick={() => controller.resolveConflict(true)}>
						{t("lockedStorage.uploadAsNewVersion", "Upload as new version")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function LockedRetentionDialog({ controller }: ControllerProps) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const id = useId();
	const target = controller.retentionTarget;
	const current = target?.version.retain_until ?? "";
	const earliest = current ? nextRetentionDay(current) : "";
	const [day, setDay] = useState("");

	useEffect(() => {
		setDay(earliest);
	}, [earliest]);

	const valid = !!day && day >= earliest;
	return (
		<Dialog
			open={!!target}
			onOpenChange={(next) => {
				if (!next) controller.closeRetention();
			}}
		>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("lockedStorage.extendRetention", "Extend retention")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"lockedStorage.extendDescription",
							"Currently locked until {{date}}. Retention can only be extended, never shortened.",
							{ date: formatLockedDate(current, language, true) },
						)}
					</DialogDescription>
				</DialogHeader>
				<form
					className="space-y-4"
					onSubmit={(event) => {
						event.preventDefault();
						if (valid)
							void controller.extendRetention(retentionFromDay(day, current));
					}}
				>
					<div className="space-y-2">
						<Label htmlFor={`${id}-until`}>
							{t("lockedStorage.lockUntil", "Lock until")}
						</Label>
						<Input
							id={`${id}-until`}
							type="date"
							min={earliest}
							value={day}
							aria-invalid={!valid}
							onChange={(event) => setDay(event.target.value)}
						/>
					</div>
					<DialogFooter>
						<Button
							type="button"
							variant="outline"
							onClick={controller.closeRetention}
						>
							{t("lockedStorage.cancel", "Cancel")}
						</Button>
						<Button type="submit" disabled={!valid || controller.mutating}>
							{t("lockedStorage.extend", "Extend")}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function LockedPurgeDialog({ controller }: ControllerProps) {
	const { t } = useTranslation("common");
	const target = controller.purgeTarget;
	const bytes =
		target?.versions.reduce((total, version) => total + version.size, 0) ?? 0;
	return (
		<AlertDialog
			open={!!target}
			onOpenChange={(next) => {
				if (!next) controller.closePurge();
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("lockedStorage.purgeTitle", "Delete expired versions?")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"lockedStorage.purgeDescription",
							"This permanently deletes the stored bytes of {{versions}} version(s) of {{name}} and frees {{size}} of storage. The record of each version stays as evidence. This cannot be undone.",
							{
								versions: target?.versions.length ?? 0,
								name: target?.item.name ?? "",
								size: humanFileSize(bytes, true),
							},
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel disabled={controller.mutating}>
						{t("lockedStorage.cancel", "Cancel")}
					</AlertDialogCancel>
					<AlertDialogAction
						className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						disabled={controller.mutating}
						onClick={(event) => {
							event.preventDefault();
							void controller.confirmPurge();
						}}
					>
						{t("lockedStorage.purgeConfirm", "Delete stored bytes")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

/** Progress, version details and every dialog the locked rows open. */
export function LockedStorageLayer({ controller }: ControllerProps) {
	const { t } = useTranslation("common");
	if (!controller.enabled) return null;
	return (
		<>
			{controller.progress.isUploading && (
				<UploadProgressPanel
					label={controller.progressLabel}
					state={controller.progress}
					cancelLabel={t("lockedStorage.cancel", "Cancel")}
					onCancel={controller.cancelUpload}
				/>
			)}
			<LockedItemDetails controller={controller} />
			<LockedUploadDialog controller={controller} />
			<LockedConflictDialog controller={controller} />
			<LockedRetentionDialog controller={controller} />
			<LockedPurgeDialog controller={controller} />
		</>
	);
}
