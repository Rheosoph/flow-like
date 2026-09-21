"use client";

import {
	Alert,
	AlertDescription,
	AlertTitle,
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
	Skeleton,
	Switch,
} from "@flow-like/flow-like-ui";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
	EyeIcon,
	EyeOffIcon,
	LockIcon,
	TriangleAlertIcon,
	UnlockIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	type IArchiveProgress,
	cancelArchiveOperation,
	describeArchiveError,
	isArchiveCancelled,
	listenArchiveProgress,
	newOperationId,
} from "../../../lib/archive-operations";
import { ArchiveProgressView } from "./ArchiveProgressView";

const ENCRYPT_STORAGE_KEY = "exportEncrypted";
const MIN_PASSWORD_LENGTH = 8;
const MAX_LISTED_SECRETS = 5;

export interface ISecretVariableRef {
	board_id: string;
	board_name: string;
	variable_id: string;
	variable_name: string;
}

export interface ITableExportStats {
	name: string;
	versions: number;
	total_bytes: number;
	history_bytes: number;
}

export interface IExportPreflight {
	total_bytes: number;
	file_count: number;
	secret_variables: ISecretVariableRef[];
	tables: ITableExportStats[];
	reclaimable_bytes: number;
	compaction_available: boolean;
}

export interface ITableCompaction {
	name: string;
	ok: boolean;
	error?: string | null;
	bytes_before: number;
	bytes_after: number;
}

export interface ICompactionReport {
	tables: ITableCompaction[];
	bytes_before: number;
	bytes_after: number;
}

export interface IExportReport {
	path: string;
	bytes_written: number;
	file_count: number;
	blob_count: number;
	compaction: ICompactionReport | null;
}

export interface ExportAppDialogProps {
	appId: string | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}

type PreflightState =
	| { status: "loading" }
	| { status: "error" }
	| { status: "ready"; data: IExportPreflight };

function readStoredEncrypt(): boolean | null {
	try {
		const saved = localStorage.getItem(ENCRYPT_STORAGE_KEY);
		return saved == null ? null : saved === "true";
	} catch {
		return null;
	}
}

function storeEncrypt(value: boolean) {
	try {
		localStorage.setItem(ENCRYPT_STORAGE_KEY, String(value));
	} catch {
		return;
	}
}

function passwordStrength(password: string): number {
	let score = 0;
	if (password.length >= MIN_PASSWORD_LENGTH) score++;
	if (/[A-Z]/.test(password) && /[a-z]/.test(password)) score++;
	if (/\d/.test(password)) score++;
	if (/[^A-Za-z0-9]/.test(password)) score++;
	return score;
}

function isPasswordValid(password: string, confirmPassword: string): boolean {
	return password.length >= MIN_PASSWORD_LENGTH && password === confirmPassword;
}

function PreflightSummary({ state }: { state: PreflightState }) {
	const { t } = useTranslation("common");
	if (state.status === "loading") return <Skeleton className="h-4 w-48" />;
	if (state.status === "error") {
		return (
			<p className="text-xs text-muted-foreground">
				{t("couldNotEstimateExportSize", "Could not estimate export size.")}
			</p>
		);
	}
	return (
		<p className="text-xs text-muted-foreground">
			{t("estimatedSizeVal", "Estimated size: {{val}}", {
				val: humanFileSize(state.data.total_bytes),
			})}
			{" · "}
			{t("countFiles", {
				defaultValue_one: "{{count}} file",
				defaultValue_other: "{{count}} files",
				count: state.data.file_count,
			})}
		</p>
	);
}

function SecretsWarning({
	secrets,
	onEncrypt,
}: {
	secrets: ISecretVariableRef[];
	onEncrypt: () => void;
}) {
	const { t } = useTranslation("common");
	const listed = secrets.slice(0, MAX_LISTED_SECRETS);
	const hidden = secrets.length - listed.length;
	return (
		<Alert variant="destructive">
			<TriangleAlertIcon />
			<AlertTitle>
				{t(
					"secretVariablesWillBeExportedInPlainText",
					"Secret variables will be exported in plain text",
				)}
			</AlertTitle>
			<AlertDescription>
				<p>
					{t(
						"anyoneWithThisFileCanReadTheirValuesEncryptTheExportToProtectThem",
						"Anyone with this file can read their values. Encrypt the export to protect them.",
					)}
				</p>
				<ul className="list-disc pl-4">
					{listed.map((secret) => (
						<li key={`${secret.board_id}:${secret.variable_id}`}>
							{secret.board_name} › {secret.variable_name}
						</li>
					))}
				</ul>
				{hidden > 0 && (
					<p>{t("lengthMore", "+{{length}} more", { length: hidden })}</p>
				)}
				<Button
					type="button"
					size="sm"
					variant="outline"
					className="mt-1"
					onClick={onEncrypt}
				>
					{t("encryptInstead", "Encrypt instead")}
				</Button>
			</AlertDescription>
		</Alert>
	);
}

function CompactionOption({
	checked,
	onCheckedChange,
}: {
	checked: boolean;
	onCheckedChange: (checked: boolean) => void;
}) {
	const { t } = useTranslation("common");
	return (
		<div className="flex items-start gap-3 rounded-lg border p-3">
			<Checkbox
				id="export-compact"
				className="mt-0.5"
				checked={checked}
				onCheckedChange={(next) => onCheckedChange(next === true)}
			/>
			<div className="grid gap-1">
				<Label htmlFor="export-compact" className="text-sm font-medium">
					{t("compactTablesBeforeExport", "Optimize tables before export")}
				</Label>
				<p className="text-xs text-muted-foreground">
					{t(
						"freesAboutValOfVersionHistoryThisRemovesTheVersionHistoryFromTheLocalTablesAndCannotBeUndone",
						"Optimize table fragments and indexes while preserving branches, tags, and version history. This can increase the export size because historical files are retained.",
					)}
				</p>
			</div>
		</div>
	);
}

function PasswordFields({
	password,
	confirmPassword,
	showPassword,
	valid,
	onPasswordChange,
	onConfirmPasswordChange,
	onToggleShowPassword,
}: {
	password: string;
	confirmPassword: string;
	showPassword: boolean;
	valid: boolean;
	onPasswordChange: (value: string) => void;
	onConfirmPasswordChange: (value: string) => void;
	onToggleShowPassword: () => void;
}) {
	const { t } = useTranslation("common");
	const strength = useMemo(() => passwordStrength(password), [password]);
	const strengthLabel =
		strength <= 1
			? t("weak", "Weak")
			: strength === 2
				? t("fair", "Fair")
				: strength === 3
					? t("good", "Good")
					: t("strong", "Strong");

	return (
		<div className="space-y-3">
			<div className="grid gap-2">
				<Label htmlFor="export-password" className="text-xs">
					{t("password", "Password")}
				</Label>
				<div className="relative">
					<Input
						id="export-password"
						type={showPassword ? "text" : "password"}
						value={password}
						onChange={(e) => onPasswordChange(e.target.value)}
						placeholder={t("enterAStrongPassword", "Enter a strong password")}
						autoFocus
					/>
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="absolute right-1 top-1 h-7 w-7"
						onClick={onToggleShowPassword}
						aria-label={
							showPassword
								? t("hidePassword", "Hide password")
								: t("showPassword", "Show password")
						}
					>
						{showPassword ? (
							<EyeOffIcon className="w-4 h-4" />
						) : (
							<EyeIcon className="w-4 h-4" />
						)}
					</Button>
				</div>
			</div>

			<div className="grid gap-2">
				<Label htmlFor="export-password-confirm" className="text-xs">
					{t("confirmPassword", "Confirm password")}
				</Label>
				<Input
					id="export-password-confirm"
					type={showPassword ? "text" : "password"}
					value={confirmPassword}
					onChange={(e) => onConfirmPasswordChange(e.target.value)}
					placeholder={t("reenterPassword", "Re-enter password")}
				/>
			</div>

			<div className="flex items-center gap-2">
				<div className="flex gap-1" aria-hidden>
					{[0, 1, 2, 3].map((i) => (
						<span
							key={i}
							className={`h-1.5 w-10 rounded ${strength > i ? "bg-primary" : "bg-muted"}`}
						/>
					))}
				</div>
				<span className="text-xs text-muted-foreground">{strengthLabel}</span>
			</div>

			{!valid && (
				<p className="text-xs text-destructive">
					{t(
						"passwordsMustMatchAndBeAtLeast8Characters",
						"Passwords must match and be at least 8 characters.",
					)}
				</p>
			)}
		</div>
	);
}

const ExportAppDialog: React.FC<ExportAppDialogProps> = ({
	appId,
	open,
	onOpenChange,
}) => {
	const { t } = useTranslation("common");
	const [encrypt, setEncryptState] = useState(false);
	const [password, setPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [showPassword, setShowPassword] = useState(false);
	const [compact, setCompact] = useState(false);
	const [exporting, setExporting] = useState(false);
	const [cancelling, setCancelling] = useState(false);
	const [progress, setProgress] = useState<IArchiveProgress | null>(null);
	const operationRef = useRef<string | null>(null);
	const [preflight, setPreflight] = useState<PreflightState>({
		status: "loading",
	});

	useEffect(() => {
		const saved = readStoredEncrypt();
		if (saved != null) setEncryptState(saved);
	}, []);

	const setEncrypt = useCallback((next: boolean) => {
		setEncryptState(next);
		storeEncrypt(next);
		if (!next) {
			setPassword("");
			setConfirmPassword("");
		}
	}, []);

	useEffect(() => {
		if (open) return;
		setPassword("");
		setConfirmPassword("");
		setShowPassword(false);
		setCompact(false);
	}, [open]);

	useEffect(() => {
		if (!open) return;
		if (!appId) {
			setPreflight({ status: "error" });
			return;
		}
		let cancelled = false;
		setPreflight({ status: "loading" });
		invoke<IExportPreflight>("get_app_export_preflight", { appId })
			.then((data) => {
				if (!cancelled) setPreflight({ status: "ready", data });
			})
			.catch((error) => {
				console.error("Export preflight error:", error);
				if (!cancelled) setPreflight({ status: "error" });
			});
		return () => {
			cancelled = true;
		};
	}, [open, appId]);

	const secrets =
		preflight.status === "ready" ? preflight.data.secret_variables : [];
	const compactionAvailable =
		preflight.status === "ready" &&
		preflight.data.compaction_available &&
		preflight.data.tables.length > 0;
	const passValid = !encrypt || isPasswordValid(password, confirmPassword);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (!next && exporting) return;
			onOpenChange(next);
		},
		[exporting, onOpenChange],
	);

	const handleCancel = useCallback(async () => {
		const operationId = operationRef.current;
		if (!operationId) return;
		setCancelling(true);
		try {
			await cancelArchiveOperation(operationId);
		} catch (error) {
			console.error("Cancel export error:", error);
			setCancelling(false);
		}
	}, []);

	const handleExport = useCallback(async () => {
		if (!appId) return;
		const operationId = newOperationId();
		const loader = toast.loading(t("exportingApp", "Exporting app..."), {
			description: t(
				"thisMayTakeAMomentPleaseWait",
				"This may take a moment, please wait.",
			),
		});
		operationRef.current = operationId;
		setExporting(true);
		setCancelling(false);
		setProgress(null);
		let unlisten: UnlistenFn | null = null;
		try {
			unlisten = await listenArchiveProgress(operationId, setProgress);
			const report = await invoke<IExportReport>("export_app_to_file", {
				appId,
				...(encrypt && password ? { password } : {}),
				compact: compact && compactionAvailable,
				operationId,
			});
			const compaction = report.compaction;
			toast.success(
				t("appExportedSuccessfully", "App exported successfully!"),
				{
					id: loader,
					description:
						compaction && compaction.bytes_before > compaction.bytes_after
							? t("compactionFreedVal", "Compaction freed {{val}}", {
									val: humanFileSize(
										compaction.bytes_before - compaction.bytes_after,
									),
								})
							: undefined,
				},
			);
			const failedTables =
				compaction?.tables.filter((table) => !table.ok) ?? [];
			if (failedTables.length > 0) {
				toast.warning(
					t(
						"someTablesCouldNotBeCompacted",
						"Some tables could not be compacted",
					),
					{ description: failedTables.map((table) => table.name).join(", ") },
				);
			}
			onOpenChange(false);
		} catch (error) {
			if (isArchiveCancelled(error)) {
				toast.info(t("exportCancelled", "Export cancelled"), { id: loader });
			} else {
				console.error("Export error:", error);
				toast.error(t("failedToExportApp", "Failed to export app"), {
					id: loader,
					description: describeArchiveError(error),
				});
			}
		} finally {
			unlisten?.();
			operationRef.current = null;
			setExporting(false);
			setCancelling(false);
			setProgress(null);
		}
	}, [appId, encrypt, password, compact, compactionAvailable, onOpenChange, t]);

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="sm:max-w-[520px]">
				<DialogHeader>
					<DialogTitle>
						{t("exportApplication", "Export Application")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"chooseHowYouWantToExportYourApp",
							"Choose how you want to export your app.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4">
					<PreflightSummary state={preflight} />

					<div className="flex items-center justify-between rounded-lg border p-3">
						<div className="flex items-center gap-3">
							{encrypt ? (
								<LockIcon className="w-4 h-4 text-primary" />
							) : (
								<UnlockIcon className="w-4 h-4 text-muted-foreground" />
							)}
							<div className="min-w-0">
								<p className="text-sm font-medium">
									{encrypt
										? t("encryptedExport", "Encrypted export")
										: t("unencryptedExport", "Unencrypted export")}
								</p>
								<p className="text-xs text-muted-foreground">
									{encrypt
										? t(
												"protectYourExportWithAPassword",
												"Protect your export with a password.",
											)
										: t(
												"quickExportWithoutEncryption",
												"Quick export without encryption.",
											)}
								</p>
							</div>
						</div>
						<div className="flex items-center gap-2">
							<Label
								htmlFor="export-encrypt"
								className="text-xs text-muted-foreground"
							>
								{t("encrypt", "Encrypt")}
							</Label>
							<Switch
								id="export-encrypt"
								checked={encrypt}
								onCheckedChange={setEncrypt}
							/>
						</div>
					</div>

					{!encrypt && secrets.length > 0 && (
						<SecretsWarning
							secrets={secrets}
							onEncrypt={() => setEncrypt(true)}
						/>
					)}

					{encrypt && (
						<PasswordFields
							password={password}
							confirmPassword={confirmPassword}
							showPassword={showPassword}
							valid={passValid}
							onPasswordChange={setPassword}
							onConfirmPasswordChange={setConfirmPassword}
							onToggleShowPassword={() => setShowPassword((s) => !s)}
						/>
					)}

					{compactionAvailable && (
						<CompactionOption checked={compact} onCheckedChange={setCompact} />
					)}

					{exporting && <ArchiveProgressView progress={progress} />}
				</div>

				<DialogFooter className="gap-2">
					<Button
						variant="outline"
						onClick={exporting ? handleCancel : () => onOpenChange(false)}
						disabled={cancelling}
					>
						{cancelling
							? t("cancelling", "Cancelling…")
							: t("cancel", "Cancel")}
					</Button>
					<Button onClick={handleExport} disabled={exporting || !passValid}>
						{exporting ? t("exporting", "Exporting...") : t("export", "Export")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};

export default ExportAppDialog;
