"use client";

import {
	AlertTriangle,
	Calendar,
	Check,
	Copy,
	Key,
	MoreVertical,
	Plus,
	Shield,
	Trash2,
} from "lucide-react";
import { useCallback, useState } from "react";
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
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
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
	ScrollArea,
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

interface TechnicalUserManagementProps {
	appId: string;
}

export function TechnicalUserManagement({
	appId,
}: Readonly<TechnicalUserManagementProps>) {
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
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<div>
							<CardTitle className="flex items-center gap-2">
								<Key className="w-5 h-5" />
								Technical Users (API Keys)
							</CardTitle>
							<CardDescription>
								Create API keys for programmatic access to this project. These
								keys can be assigned roles with specific permissions.
							</CardDescription>
						</div>
						<Button
							onClick={() => setShowCreateDialog(true)}
							className="bg-linear-to-r from-primary to-tertiary hover:from-primary/50 hover:to-tertiary/50"
						>
							<Plus className="w-4 h-4 mr-2" />
							Create API Key
						</Button>
					</div>
				</CardHeader>
				<CardContent>
					{!apiKeys.data || apiKeys.data.length === 0 ? (
						<EmptyState
							icons={[Key]}
							title="No API Keys"
							description="Create an API key to enable programmatic access to this project."
						/>
					) : (
						<ScrollArea className="h-[400px]">
							<div className="space-y-3 pr-4">
								{apiKeys.data.map((key) => (
									<ApiKeyCard
										key={key.id}
										apiKey={key}
										roles={roles.data?.[1] ?? []}
										onDelete={handleDelete}
									/>
								))}
							</div>
						</ScrollArea>
					)}
				</CardContent>
			</Card>

			{/* Create Dialog */}
			<Dialog open={showCreateDialog} onOpenChange={setShowCreateDialog}>
				<DialogContent className="sm:max-w-md">
					<DialogHeader>
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
							<Key className="h-6 w-6 text-primary" />
						</div>
						<DialogTitle className="text-center text-xl">
							Create API Key
						</DialogTitle>
						<DialogDescription className="text-center">
							Create a new API key for programmatic access
						</DialogDescription>
					</DialogHeader>

					<div className="space-y-4 py-4">
						<div className="space-y-2">
							<Label htmlFor="name">Name *</Label>
							<Input
								id="name"
								placeholder="e.g., CI/CD Pipeline"
								value={name}
								onChange={(e) => setName(e.target.value)}
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="description">Description</Label>
							<Textarea
								id="description"
								placeholder="What is this API key used for?"
								value={description}
								onChange={(e) => setDescription(e.target.value)}
								className="min-h-[60px] resize-none"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="role">Role</Label>
							<Select value={selectedRoleId} onValueChange={setSelectedRoleId}>
								<SelectTrigger>
									<SelectValue placeholder="Select a role (optional)" />
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
								The role determines what permissions this API key has
							</p>
						</div>

						<div className="space-y-2">
							<Label htmlFor="validUntil">Expiration Date</Label>
							<Input
								id="validUntil"
								type="datetime-local"
								value={validUntil}
								onChange={(e) => setValidUntil(e.target.value)}
							/>
							<p className="text-xs text-muted-foreground">
								Leave empty for no expiration
							</p>
						</div>
					</div>

					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setShowCreateDialog(false)}
						>
							Cancel
						</Button>
						<Button
							onClick={handleCreate}
							disabled={isCreating || !name.trim()}
						>
							{isCreating ? "Creating..." : "Create API Key"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Show Key Dialog */}
			<Dialog open={showKeyDialog} onOpenChange={setShowKeyDialog}>
				<DialogContent className="sm:max-w-lg">
					<DialogHeader>
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/20">
							<Check className="h-6 w-6 text-green-600 dark:text-green-400" />
						</div>
						<DialogTitle className="text-center text-xl">
							API Key Created
						</DialogTitle>
						<DialogDescription className="text-center">
							Copy your API key now. You won&apos;t be able to see it again!
						</DialogDescription>
					</DialogHeader>

					<div className="space-y-4 py-4">
						<div className="rounded-lg border bg-muted/50 p-4">
							<div className="flex items-center justify-between gap-2">
								<code className="flex-1 break-all text-sm font-mono">
									{newApiKey}
								</code>
								<Button variant="ghost" size="icon" onClick={copyApiKey}>
									<Copy className="h-4 w-4" />
								</Button>
							</div>
						</div>

						<div className="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-3 dark:border-amber-900/50 dark:bg-amber-900/20">
							<AlertTriangle className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
							<div className="text-sm text-amber-800 dark:text-amber-200">
								<p className="font-medium">Important</p>
								<p>
									This is the only time you&apos;ll see this API key. Make sure
									to copy it and store it securely. Use the{" "}
									<code className="rounded bg-amber-200/50 px-1 dark:bg-amber-800/50">
										x-api-key
									</code>{" "}
									header to authenticate requests.
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
							Done
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
	const [showDeleteDialog, setShowDeleteDialog] = useState(false);
	const role = roles.find((r) => r.id === apiKey.role_id);

	const isExpired = apiKey.valid_until
		? apiKey.valid_until * 1000 < Date.now()
		: false;
	return (
		<>
			<div
				className={`flex items-center justify-between p-4 border rounded-lg ${
					isExpired
						? "border-red-200 bg-red-50/50 dark:border-red-900/50 dark:bg-red-900/10"
						: "hover:bg-muted/50"
				} transition-colors`}
			>
				<div className="flex items-center gap-3 min-w-0 flex-1">
					<div
						className={`flex h-10 w-10 items-center justify-center rounded-full ${
							isExpired ? "bg-red-100 dark:bg-red-900/30" : "bg-primary/10"
						}`}
					>
						<Key
							className={`h-5 w-5 ${isExpired ? "text-red-600 dark:text-red-400" : "text-primary"}`}
						/>
					</div>
					<div className="min-w-0 flex-1">
						<div className="flex items-center gap-2">
							<h3 className="font-medium text-sm truncate">{apiKey.name}</h3>
							{isExpired && (
								<span className="text-xs px-2 py-0.5 rounded-full bg-red-100 text-red-600 dark:bg-red-900/30 dark:text-red-400">
									Expired
								</span>
							)}
						</div>
						<div className="flex items-center gap-3 text-xs text-muted-foreground">
							{role && (
								<span className="flex items-center gap-1">
									<Shield className="h-3 w-3" />
									{role.name}
								</span>
							)}
							<UserProfileLink
								userId={apiKey.creator_user_id}
								name={apiKey.creator_display_name}
								email={apiKey.creator_email}
								fallbackLabel="Unknown owner"
								className="max-w-48"
								muted
							/>
							{apiKey.valid_until && (
								<span className="flex items-center gap-1">
									<Calendar className="h-3 w-3" />
									{isExpired ? "Expired " : "Expires "}
									{new Date(apiKey.valid_until * 1000).toLocaleDateString()}
								</span>
							)}
							<span>
								Created{" "}
								{new Date(apiKey.created_at * 1000).toLocaleDateString()}
							</span>
						</div>
						{apiKey.description && (
							<p className="text-xs text-muted-foreground mt-1 truncate">
								{apiKey.description}
							</p>
						)}
					</div>
				</div>

				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button variant="ghost" size="icon">
							<MoreVertical className="h-4 w-4" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end">
						<DropdownMenuItem
							className="text-destructive focus:text-destructive"
							onClick={() => setShowDeleteDialog(true)}
						>
							<Trash2 className="h-4 w-4 mr-2" />
							Delete
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>
			</div>

			<AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Delete API Key</AlertDialogTitle>
						<AlertDialogDescription>
							Are you sure you want to delete the API key &quot;{apiKey.name}
							&quot;? This action cannot be undone and any applications using
							this key will lose access.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Cancel</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								onDelete(apiKey.id, apiKey.name);
								setShowDeleteDialog(false);
							}}
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						>
							Delete
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}
