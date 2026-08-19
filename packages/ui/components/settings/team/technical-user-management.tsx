"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	CalendarIcon,
	CheckIcon,
	ClockIcon,
	CopyIcon,
	KeyIcon,
	MoreVerticalIcon,
	PlusIcon,
	ShieldIcon,
	Trash2Icon,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import type { IBackendRole, ITechnicalUser } from "../../..";
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
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	EmptyState,
	Input,
	Label,
	RolePermissions,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Textarea,
	UserProfileLink,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "../../../";
import {
	SectionHeading,
	StatusChip,
	TEAM_ACTION_GRADIENT,
	TEAM_ROW_DESCRIPTION,
	TEAM_ROW_META,
	TEAM_ROW_TITLE,
	TeamCallout,
	TeamHint,
	TeamRowActions,
	TeamRowIcon,
	TeamSection,
	teamRowClass,
} from "./team-shared";

interface TechnicalUserManagementProps {
	appId: string;
}

export function TechnicalUserManagement({
	appId,
}: Readonly<TechnicalUserManagementProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const apiKeys = useInvoke(
		backend.apiKeyState.getApiKeys,
		backend.apiKeyState,
		[appId],
	);
	const roles = useInvoke(backend.roleState.getRoles, backend.roleState, [
		appId,
	]);

	const [showCreateDialog, setShowCreateDialog] = useState(false);
	const [showKeyDialog, setShowKeyDialog] = useState(false);
	const [newApiKey, setNewApiKey] = useState<string>("");
	const [newKeyName, setNewKeyName] = useState("");

	// Form state
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [selectedRoleId, setSelectedRoleId] = useState<string | undefined>();
	const [validUntil, setValidUntil] = useState<string>("");
	const [isCreating, setIsCreating] = useState(false);

	const availableRoles =
		roles.data?.[1].filter((role) => {
			const perm = new RolePermissions(BigInt(role.permissions));
			return !perm.contains(RolePermissions.Owner);
		}) ?? [];

	const expiredCount = useMemo(() => {
		const now = Date.now();
		return (apiKeys.data ?? []).filter(
			(key) => key.valid_until && key.valid_until * 1000 < now,
		).length;
	}, [apiKeys.data]);

	const handleCreate = useCallback(async () => {
		if (!name.trim()) {
			toast.error("Please enter a name for the API key");
			return;
		}

		try {
			setIsCreating(true);
			const result = await backend.apiKeyState.createApiKey(appId, {
				name: name.trim(),
				description: description.trim() || undefined,
				role_id: selectedRoleId,
				valid_until: validUntil
					? Math.floor(new Date(validUntil).getTime() / 1000)
					: undefined,
			});

			setNewApiKey(result.api_key);
			setNewKeyName(result.name);
			setShowCreateDialog(false);
			setShowKeyDialog(true);

			// Reset form
			setName("");
			setDescription("");
			setSelectedRoleId(undefined);
			setValidUntil("");

			invalidate(backend.apiKeyState.getApiKeys, [appId]);
			toast.success("API key created successfully!");
		} catch (error) {
			console.error(error);
			toast.error("Failed to create API key");
		} finally {
			setIsCreating(false);
		}
	}, [
		appId,
		backend,
		name,
		description,
		selectedRoleId,
		validUntil,
		invalidate,
	]);

	const handleDelete = useCallback(
		async (keyId: string, keyName: string) => {
			try {
				await backend.apiKeyState.deleteApiKey(appId, keyId);
				invalidate(backend.apiKeyState.getApiKeys, [appId]);
				toast.success(`API key "${keyName}" deleted successfully`);
			} catch (error) {
				console.error(error);
				toast.error("Failed to delete API key");
			}
		},
		[appId, backend, invalidate],
	);

	const copyApiKey = useCallback(() => {
		navigator.clipboard.writeText(newApiKey);
		toast.success("API key copied to clipboard!");
	}, [newApiKey]);

	return (
		<div className="space-y-8">
			<TeamSection>
				<SectionHeading
					icon={KeyIcon}
					title={t("apiKeys", "API keys")}
					count={apiKeys.data?.length ?? 0}
					description={t(
						"forScriptsAndServicesAKeyActsWithTheRoleYouGiveItNothingMore",
						"For scripts and services. A key acts with the role you give it — nothing more.",
					)}
					actions={
						<Button
							size="sm"
							className={TEAM_ACTION_GRADIENT}
							onClick={() => setShowCreateDialog(true)}
						>
							<PlusIcon className="size-4" />
							{t("newKey", "New key")}
						</Button>
					}
				/>

				{expiredCount > 0 && (
					<TeamCallout icon={ClockIcon} tone="attention">
						{t(
							"countKeysHaveExpiredAnythingStillCallingWithThemIsBeingRejected",
							{
								defaultValue_one:
									"1 key has expired. Anything still calling with it is being rejected.",
								defaultValue_other:
									"{{count}} keys have expired. Anything still calling with them is being rejected.",
								count: expiredCount,
							},
						)}
					</TeamCallout>
				)}

				{!apiKeys.data || apiKeys.data.length === 0 ? (
					<EmptyState
						className="max-w-full"
						icons={[KeyIcon]}
						title={t("noApiKeys", "No API Keys")}
						description={t(
							"createAnApiKeyToEnableProgrammaticAccessToThisProject",
							"Create an API key to enable programmatic access to this project.",
						)}
					/>
				) : (
					<div className="flex flex-col gap-2">
						{apiKeys.data.map((key) => (
							<ApiKeyCard
								key={key.id}
								apiKey={key}
								roles={roles.data?.[1] ?? []}
								onDelete={handleDelete}
							/>
						))}
					</div>
				)}

				<TeamHint>
					{t("keysAuthenticateWithThe", "Keys authenticate with the")}{" "}
					<code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">
						x-api-key
					</code>{" "}
					{t(
						"headerTheSecretIsShownOnceWhenTheKeyIsCreatedItCannotBeReadAgainAfterwards",
						"header. The secret is shown once when the key is created — it cannot be read again afterwards.",
					)}
				</TeamHint>
			</TeamSection>

			{/* Create Dialog */}
			<Dialog open={showCreateDialog} onOpenChange={setShowCreateDialog}>
				<DialogContent className="sm:max-w-md">
					<DialogHeader>
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
							<KeyIcon className="h-6 w-6 text-primary" />
						</div>
						<DialogTitle className="text-center text-xl">
							{t("createApiKey", "Create API Key")}
						</DialogTitle>
						<DialogDescription className="text-center">
							{`Create a new API key for programmatic access`}
						</DialogDescription>
					</DialogHeader>

					<div className="space-y-4 py-4">
						<div className="space-y-2">
							<Label htmlFor="name">{t("name", "Name *")}</Label>
							<Input
								id="name"
								placeholder={t("egCicdPipeline", "e.g., CI/CD Pipeline")}
								value={name}
								onChange={(e) => setName(e.target.value)}
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="description">
								{t("description", "Description")}
							</Label>
							<Textarea
								id="description"
								placeholder={t(
									"whatIsThisApiKeyUsedFor",
									"What is this API key used for?",
								)}
								value={description}
								onChange={(e) => setDescription(e.target.value)}
								className="min-h-20 resize-none"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="role">Role</Label>
							<Select value={selectedRoleId} onValueChange={setSelectedRoleId}>
								<SelectTrigger>
									<SelectValue
										placeholder={t(
											"selectARoleOptional",
											"Select a role (optional)",
										)}
									/>
								</SelectTrigger>
								<SelectContent>
									{availableRoles.map((role) => (
										<SelectItem key={role.id} value={role.id}>
											{role.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<p className="text-xs text-muted-foreground">
								{t(
									"theRoleDeterminesWhatPermissionsThisApiKeyHas",
									"The role determines what permissions this API key has",
								)}
							</p>
						</div>

						<div className="space-y-2">
							<Label htmlFor="validUntil">
								{t("expirationDate", "Expiration Date")}
							</Label>
							<Input
								id="validUntil"
								type="datetime-local"
								value={validUntil}
								onChange={(e) => setValidUntil(e.target.value)}
							/>
							<p className="text-xs text-muted-foreground">
								{t(
									"leaveEmptyForNoExpiration",
									"Leave empty for no expiration",
								)}
							</p>
						</div>
					</div>

					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setShowCreateDialog(false)}
						>
							{t("cancel", "Cancel")}
						</Button>
						<Button
							onClick={handleCreate}
							disabled={isCreating || !name.trim()}
						>
							{isCreating ? "Creating..." : t("createApiKey", "Create API Key")}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Show Key Dialog */}
			<Dialog open={showKeyDialog} onOpenChange={setShowKeyDialog}>
				<DialogContent className="sm:max-w-lg">
					<DialogHeader>
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/20">
							<CheckIcon className="h-6 w-6 text-green-600 dark:text-green-400" />
						</div>
						<DialogTitle className="text-center text-xl">
							{t("apiKeyCreated", "API Key Created")}
						</DialogTitle>
						<DialogDescription className="text-center">
							{t(
								"copyYourApiKeyNowYouWonapostBeAbleToSeeItAgain",
								"Copy your API key now. You won't be able to see it again!",
							)}
						</DialogDescription>
					</DialogHeader>

					<div className="space-y-4 py-4">
						<div className="rounded-lg border bg-muted/50 p-4">
							<div className="flex items-center justify-between gap-2">
								<code className="flex-1 break-all text-sm font-mono">
									{newApiKey}
								</code>
								<Button variant="ghost" size="icon" onClick={copyApiKey}>
									<CopyIcon className="h-4 w-4" />
								</Button>
							</div>
						</div>

						<div className="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-3 dark:border-amber-900/50 dark:bg-amber-900/20">
							<AlertTriangleIcon className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
							<div className="text-sm text-amber-800 dark:text-amber-200">
								<p className="font-medium">{t("important", "Important")}</p>
								<p>
									{t(
										"thisIsTheOnlyTimeYouaposllSeeThisApiKeyMakeSureToCopyItAndStoreItSecurelyUseThe",
										"This is the only time you'll see this API key. Make sure to copy it and store it securely. Use the",
									)}{" "}
									<code className="rounded bg-amber-200/50 px-1 dark:bg-amber-800/50">
										x-api-key
									</code>{" "}
									{t(
										"headerToAuthenticateRequests",
										"header to authenticate requests.",
									)}
								</p>
							</div>
						</div>
					</div>

					<DialogFooter>
						<Button
							onClick={() => {
								setShowKeyDialog(false);
								setNewApiKey("");
								setNewKeyName("");
							}}
						>
							{t("done", "Done")}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}

interface ApiKeyCardProps {
	apiKey: ITechnicalUser;
	roles: IBackendRole[];
	onDelete: (id: string, name: string) => void;
}

function ApiKeyCard({ apiKey, roles, onDelete }: Readonly<ApiKeyCardProps>) {
	const { t } = useTranslation("settings");
	const [showDeleteDialog, setShowDeleteDialog] = useState(false);
	const role = roles.find((r) => r.id === apiKey.role_id);

	const isExpired = apiKey.valid_until
		? apiKey.valid_until * 1000 < Date.now()
		: false;
	return (
		<>
			<div className={teamRowClass({ muted: isExpired })}>
				<TeamRowIcon icon={KeyIcon} />
				<div className="min-w-0 flex-1">
					<div className={TEAM_ROW_TITLE}>
						<span className="truncate">{apiKey.name}</span>
						{isExpired && (
							<StatusChip tone="danger" pip>
								{t("expired", "Expired")}
							</StatusChip>
						)}
					</div>
					<div className={TEAM_ROW_META}>
						{role && <StatusChip icon={ShieldIcon}>{role.name}</StatusChip>}
						<span className="flex items-center gap-1">
							<CalendarIcon className="size-3.5" />
							{apiKey.valid_until
								? `${isExpired ? "Expired" : "Expires"} ${new Date(
										apiKey.valid_until * 1000,
									).toLocaleDateString()}`
								: t("noExpiry", "No expiry")}
						</span>
						<span>
							{t("created", "Created")}{" "}
							{new Date(apiKey.created_at * 1000).toLocaleDateString()}
						</span>
						<UserProfileLink
							userId={apiKey.creator_user_id}
							name={apiKey.creator_display_name}
							email={apiKey.creator_email}
							fallbackLabel="Unknown owner"
							className="max-w-48"
							muted
						/>
					</div>
					{apiKey.description && (
						<p className={TEAM_ROW_DESCRIPTION}>{apiKey.description}</p>
					)}
				</div>

				<TeamRowActions>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button variant="ghost" size="icon" className="size-8">
								<MoreVerticalIcon className="size-4" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							<DropdownMenuItem
								variant="destructive"
								onClick={() => setShowDeleteDialog(true)}
							>
								<Trash2Icon className="size-4" />
								{t("delete", "Delete")}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</TeamRowActions>
			</div>

			<AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("deleteApiKey", "Delete API Key")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"areYouSureYouWantToDeleteTheApiKeyQuotnameQuotThisActionCannotBeUndoneAndAnyApplicationsUsingThisKeyWillLoseAccess",
								'Are you sure you want to delete the API key "{{name}} "? This action cannot be undone and any applications using this key will lose access.',
								{ name: apiKey.name },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								onDelete(apiKey.id, apiKey.name);
								setShowDeleteDialog(false);
							}}
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						>
							{t("delete", "Delete")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}
