"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarIcon,
	CheckIcon,
	CopyIcon,
	KeyRoundIcon,
	PlusIcon,
	ShieldAlertIcon,
	ShieldCheckIcon,
	Trash2Icon,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../hooks/use-invoke";
import { parseDateValue } from "../lib/date";
import { cn } from "../lib/utils";
import { useBackend } from "../state/backend-state";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Calendar } from "./ui/calendar";
import { Card } from "./ui/card";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "./ui/dialog";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "./ui/select";
import { Separator } from "./ui/separator";

interface PAT {
	id: string;
	name: string;
	created_at: string;
	valid_until: string | null;
	permission: number;
}

function isExpired(validUntil: string | null): boolean {
	const parsed = parseDateValue(validUntil);
	if (!parsed) return false;
	return parsed.getTime() < Date.now();
}

function formatDate(value: Date | string, language: string): string {
	const parsed = parseDateValue(value);
	if (!parsed) return "";
	return new Intl.DateTimeFormat(language, { dateStyle: "medium" }).format(
		parsed,
	);
}

function TokenRow({
	pat,
	onDelete,
}: Readonly<{ pat: PAT; onDelete: (id: string, name: string) => void }>) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const getPermissionLabel = (permission: number) =>
		permission === 1
			? t("readOnly", "Read Only")
			: permission === 2
				? t("readWrite", "Read & Write")
				: permission === 4
					? t("admin", "Admin")
					: t("unknown", "Unknown");
	const [copied, setCopied] = useState(false);
	const expired = isExpired(pat.valid_until);

	const copyId = useCallback(() => {
		navigator.clipboard.writeText(pat.id);
		setCopied(true);
		toast.success(t("tokenIdCopied", "Token ID copied"));
		setTimeout(() => setCopied(false), 2000);
	}, [pat.id, t]);

	return (
		<div
			className={cn(
				"group flex items-center justify-between gap-4 rounded-lg border px-4 py-3 transition-colors hover:bg-muted/40",
				expired && "opacity-50",
			)}
		>
			<div className="flex items-center gap-3 min-w-0">
				<KeyRoundIcon
					className={cn(
						"h-4 w-4 shrink-0",
						expired ? "text-destructive" : "text-muted-foreground",
					)}
				/>
				<div className="min-w-0">
					<div className="flex items-center gap-2">
						<span className="font-medium truncate text-sm">{pat.name}</span>
						{expired && (
							<Badge variant="destructive" className="text-[10px] px-1.5 py-0">
								{t("expired", "Expired")}
							</Badge>
						)}
					</div>
					<div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground mt-0.5">
						<span>{getPermissionLabel(pat.permission)}</span>
						<span className="text-border">|</span>
						<span>
							{t("createdOnDate", "Created {{date}}", {
								date: formatDate(pat.created_at, language),
							})}
						</span>
						{pat.valid_until && (
							<>
								<span className="text-border">|</span>
								<span>
									{t("expiresOnDate", "Expires {{date}}", {
										date: formatDate(pat.valid_until, language),
									})}
								</span>
							</>
						)}
						{!pat.valid_until && (
							<>
								<span className="text-border">|</span>
								<span>{t("noExpiry", "No expiry")}</span>
							</>
						)}
					</div>
				</div>
			</div>

			<div className="flex items-center gap-1 shrink-0 opacity-100 md:opacity-0 md:group-hover:opacity-100 transition-opacity">
				<Button
					variant="ghost"
					size="icon"
					className="h-10 w-10 md:h-8 md:w-8"
					onClick={copyId}
				>
					{copied ? (
						<CheckIcon className="h-3.5 w-3.5 text-green-500" />
					) : (
						<CopyIcon className="h-3.5 w-3.5" />
					)}
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-10 w-10 md:h-8 md:w-8 text-destructive hover:text-destructive"
					onClick={() => onDelete(pat.id, pat.name)}
				>
					<Trash2Icon className="h-3.5 w-3.5" />
				</Button>
			</div>
		</div>
	);
}

function TokenRevealDialog({
	open,
	token,
	permission,
	onClose,
}: Readonly<{
	open: boolean;
	token: string;
	permission: number;
	onClose: () => void;
}>) {
	const { t } = useTranslation("common");
	const getPermissionLabel = (permission: number) =>
		permission === 1
			? t("readOnly", "Read Only")
			: permission === 2
				? t("readWrite", "Read & Write")
				: permission === 4
					? t("admin", "Admin")
					: t("unknown", "Unknown");
	const [copied, setCopied] = useState(false);

	const copyToken = useCallback(() => {
		navigator.clipboard.writeText(token);
		setCopied(true);
		toast.success(t("tokenCopiedToClipboard", "Token copied to clipboard"));
		setTimeout(() => setCopied(false), 2000);
	}, [token, t]);

	return (
		<Dialog open={open} onOpenChange={() => onClose()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<CheckIcon className="h-5 w-5 text-green-500" />
						{t("tokenCreated", "Token created")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"copyThisTokenNowItWonapostBeShownAgain",
							"Copy this token now — it won't be shown again.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4">
					<div className="rounded-lg bg-muted/50 border p-3 space-y-2">
						<code className="block text-sm font-mono break-all select-all leading-relaxed">
							{token}
						</code>
						<Button
							variant="secondary"
							size="sm"
							onClick={copyToken}
							className="w-full gap-2"
						>
							{copied ? (
								<CheckIcon className="h-3.5 w-3.5" />
							) : (
								<CopyIcon className="h-3.5 w-3.5" />
							)}
							{copied ? t("copied", "Copied!") : t("copyToken", "Copy Token")}
						</Button>
					</div>

					<div className="flex items-start gap-2.5 rounded-lg bg-amber-500/10 border border-amber-500/20 p-3">
						<ShieldAlertIcon className="h-4 w-4 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
						<p className="text-xs text-muted-foreground leading-relaxed">
							{t(
								"tokenPermissionAccessWarning",
								"This token has {{permission}} access. Store it securely and never share it in public.",
								{ permission: getPermissionLabel(permission) },
							)}
						</p>
					</div>
				</div>

				<DialogFooter>
					<Button onClick={onClose}>{t("done", "Done")}</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function CreateTokenDialog({
	open,
	onOpenChange,
	onCreated,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onCreated: (token: string, permission: number) => void;
}>) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const permissionLevels = [
		{
			value: 1,
			label: t("readOnly", "Read Only"),
			description: t("viewAccessOnly", "View access only"),
		},
		{
			value: 2,
			label: t("readWrite", "Read & Write"),
			description: t("viewAndModifyAccess", "View and modify access"),
		},
		{
			value: 4,
			label: t("admin", "Admin"),
			description: t("fullAdministrativeAccess", "Full administrative access"),
		},
	];
	const backend = useBackend();
	const [name, setName] = useState("");
	const [expiration, setExpiration] = useState<Date | undefined>(undefined);
	const [permission, setPermission] = useState<number>(1);
	const [creating, setCreating] = useState(false);

	const reset = useCallback(() => {
		setName("");
		setExpiration(undefined);
		setPermission(1);
	}, []);

	const handleCreate = useCallback(async () => {
		if (!name.trim()) return;
		try {
			setCreating(true);
			const result = await backend.userState.createPAT(
				name,
				expiration,
				permission,
			);
			reset();
			onOpenChange(false);
			onCreated(result.pat, result.permission);
		} catch {
			toast.error(t("failedToCreateToken", "Failed to create token"));
		} finally {
			setCreating(false);
		}
	}, [
		name,
		expiration,
		permission,
		backend.userState,
		reset,
		onOpenChange,
		onCreated,
		t,
	]);

	return (
		<Dialog
			open={open}
			onOpenChange={(v) => {
				if (!v) reset();
				onOpenChange(v);
			}}
		>
			<DialogContent className="max-w-md">
				<DialogHeader>
					<DialogTitle>{t("createToken", "Create Token")}</DialogTitle>
					<DialogDescription>
						{t(
							"generateANewPersonalAccessToken",
							"Generate a new personal access token.",
						)}
					</DialogDescription>
				</DialogHeader>

				<DialogBody className="space-y-4">
					<div className="space-y-1.5">
						<Label htmlFor="pat-name" className="text-sm">
							{t("tokenName", "Token Name *")}
						</Label>
						<Input
							id="pat-name"
							placeholder={t("egCicdPipeline", "e.g. CI/CD Pipeline")}
							value={name}
							onChange={(e) => setName(e.target.value)}
							autoFocus
						/>
					</div>

					<div className="space-y-1.5">
						<Label className="text-sm">{t("permission", "Permission")}</Label>
						<Select
							value={permission.toString()}
							onValueChange={(v) => setPermission(Number.parseInt(v))}
						>
							<SelectTrigger className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{permissionLevels.map((level) => (
									<SelectItem key={level.value} value={level.value.toString()}>
										<div className="flex flex-col">
											<span className="font-medium">{level.label}</span>
											<span className="text-xs text-muted-foreground">
												{level.description}
											</span>
										</div>
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>

					<div className="space-y-1.5">
						<Label className="text-sm">{t("expiration", "Expiration")}</Label>
						<Popover>
							<PopoverTrigger asChild>
								<Button
									variant="outline"
									className={cn(
										"w-full justify-start text-left font-normal",
										!expiration && "text-muted-foreground",
									)}
								>
									<CalendarIcon className="mr-2 h-4 w-4" />
									{expiration
										? formatDate(expiration, language)
										: t("noExpiration", "No expiration")}
								</Button>
							</PopoverTrigger>
							<PopoverContent className="w-auto p-0" align="start">
								<Calendar
									mode="single"
									selected={expiration}
									onSelect={setExpiration}
									disabled={(date) => date < new Date()}
									initialFocus
								/>
								{expiration && (
									<div className="p-2 pt-0">
										<Button
											variant="ghost"
											size="sm"
											onClick={() => setExpiration(undefined)}
											className="w-full text-xs"
										>
											{t("clear", "Clear")}
										</Button>
									</div>
								)}
							</PopoverContent>
						</Popover>
					</div>
				</DialogBody>

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => {
							reset();
							onOpenChange(false);
						}}
						disabled={creating}
					>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={handleCreate} disabled={creating || !name.trim()}>
						{creating ? t("creating", "Creating...") : t("create", "Create")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function PatManagement() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const pats = useInvoke(backend.userState.getPATs, backend.userState, []);

	const [showCreate, setShowCreate] = useState(false);
	const [revealToken, setRevealToken] = useState<string>("");
	const [revealPermission, setRevealPermission] = useState<number>(1);

	const handleCreated = useCallback(
		async (token: string, permission: number) => {
			setRevealToken(token);
			setRevealPermission(permission);
			await pats.refetch();
			toast.success(t("tokenCreated", "Token created"));
		},
		[pats, t],
	);

	const handleDelete = useCallback(
		async (id: string, name: string) => {
			if (
				!confirm(
					t(
						"deleteNameThisCannotBeUndone",
						'Delete "{{name}}"? This cannot be undone.',
						{ name },
					),
				)
			) {
				return;
			}
			try {
				await backend.userState.deletePAT(id);
				await pats.refetch();
				toast.success(t("tokenDeleted", "Token deleted"));
			} catch {
				toast.error(t("failedToDeleteToken", "Failed to delete token"));
			}
		},
		[backend.userState, pats, t],
	);

	const sortedPats = useMemo(() => {
		if (!pats.data) return [];
		return [...pats.data].sort((a, b) => {
			const aExpired = isExpired(a.valid_until);
			const bExpired = isExpired(b.valid_until);
			if (aExpired !== bExpired) return aExpired ? 1 : -1;
			return (
				(parseDateValue(b.created_at)?.getTime() ?? 0) -
				(parseDateValue(a.created_at)?.getTime() ?? 0)
			);
		});
	}, [pats.data]);

	return (
		<div className="flex h-full grow flex-col overflow-hidden">
			<div className="mx-auto w-full max-w-3xl grow overflow-y-auto px-6 py-8 space-y-6">
				{/* Header */}
				<div className="space-y-1">
					<h1 className="text-2xl font-semibold tracking-tight">
						{t("accessTokens", "Access Tokens")}
					</h1>
					<p className="text-sm text-muted-foreground">
						{t(
							"managePersonalAccessTokensForApiIntegrationsAndExternalServices",
							"Manage personal access tokens for API integrations and external services.",
						)}
					</p>
				</div>

				<Separator />

				{/* Actions */}
				<div className="flex items-center justify-between">
					<p className="text-sm text-muted-foreground">
						{t("countTokens", "{{count}} token", {
							count: pats.data?.length ?? 0,
						})}
					</p>
					<Button
						size="sm"
						onClick={() => setShowCreate(true)}
						className="gap-1.5"
					>
						<PlusIcon className="h-4 w-4" />
						{t("newToken", "New Token")}
					</Button>
				</div>

				{/* Token list */}
				{pats.isLoading ? (
					<div className="space-y-2">
						{[1, 2, 3].map((i) => (
							<div
								key={i}
								className="h-16 rounded-lg border animate-pulse bg-muted/30"
							/>
						))}
					</div>
				) : sortedPats.length === 0 ? (
					<Card className="flex flex-col items-center justify-center py-16 text-center border-dashed">
						<div className="rounded-full bg-muted p-3 mb-4">
							<KeyRoundIcon className="h-6 w-6 text-muted-foreground" />
						</div>
						<h3 className="font-medium mb-1">
							{t("noTokensYet", "No tokens yet")}
						</h3>
						<p className="text-sm text-muted-foreground mb-4 max-w-xs">
							{t(
								"createYourFirstAccessTokenToIntegrateWithExternalServices",
								"Create your first access token to integrate with external services.",
							)}
						</p>
						<Button
							variant="outline"
							size="sm"
							onClick={() => setShowCreate(true)}
							className="gap-1.5"
						>
							<PlusIcon className="h-4 w-4" />
							{t("createToken", "Create Token")}
						</Button>
					</Card>
				) : (
					<div className="space-y-1.5">
						{sortedPats.map((pat) => (
							<TokenRow key={pat.id} pat={pat} onDelete={handleDelete} />
						))}
					</div>
				)}

				{/* Security note */}
				{sortedPats.length > 0 && (
					<div className="flex items-start gap-2.5 rounded-lg border p-3">
						<ShieldCheckIcon className="h-4 w-4 text-muted-foreground mt-0.5 shrink-0" />
						<p className="text-xs text-muted-foreground leading-relaxed">
							{t(
								"tokensGrantAccessToYourAccountRegularlyReviewAndRevokeTokensYouNoLongerUseDeleteCompromisedTokensImmediately",
								"Tokens grant access to your account. Regularly review and revoke tokens you no longer use. Delete compromised tokens immediately.",
							)}
						</p>
					</div>
				)}
			</div>

			{/* Dialogs */}
			<CreateTokenDialog
				open={showCreate}
				onOpenChange={setShowCreate}
				onCreated={handleCreated}
			/>
			<TokenRevealDialog
				open={!!revealToken}
				token={revealToken}
				permission={revealPermission}
				onClose={() => setRevealToken("")}
			/>
		</div>
	);
}
