"use client";

import { useDebounce } from "@uidotdev/usehooks";
import {
	ArrowDownLeftIcon,
	ArrowUpRightIcon,
	BlocksIcon,
	CheckIcon,
	ClockIcon,
	ListIcon,
	MoreVerticalIcon,
	PlusIcon,
	RefreshCwIcon,
	SearchIcon,
	SendIcon,
	SettingsIcon,
	ShieldIcon,
	Trash2Icon,
	WaypointsIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import type { IAppConnection, IBackendRole } from "../../..";
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
	Avatar,
	AvatarFallback,
	AvatarImage,
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
	useBackend,
	useInvalidateInvoke,
	useInvoke,
	useSearch,
} from "../../..";
import type { IApp } from "../../../lib/schema/app/app";
import type { IMetadata } from "../../../lib/schema/bit/bit";
import {
	CapabilityBadges,
	deriveConnectionCapabilities,
} from "../connections/capabilities";
import { ProcessGraph } from "../connections/process-graph";
import {
	SectionHeading,
	StatusChip,
	TEAM_ACTION_GRADIENT,
	TEAM_ROW_DESCRIPTION,
	TEAM_ROW_META,
	TEAM_ROW_TITLE,
	TeamRowActions,
	TeamRowNote,
	TeamSection,
	teamRowClass,
} from "./team-shared";

interface AppConnectionManagementProps {
	appId: string;
}

export function AppConnectionManagement({
	appId,
}: Readonly<AppConnectionManagementProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [view, setView] = useState<"list" | "graph">("list");
	const [days, setDays] = useState(30);
	const connections = useInvoke(
		backend.teamState.getAppConnections,
		backend.teamState,
		[appId],
	);
	const roles = useInvoke(backend.roleState.getRoles, backend.roleState, [
		appId,
	]);
	const graph = useInvoke(
		backend.teamState.getConnectionGraph,
		backend.teamState,
		[appId, days],
		view === "graph",
	);

	const cases = useInvoke(
		backend.teamState.getProcessCases,
		backend.teamState,
		[appId, days],
		view === "graph",
	);

	const availableRoles = useMemo(
		() =>
			roles.data?.[1].filter((role) => {
				const perm = new RolePermissions(BigInt(role.permissions));
				return (
					!perm.contains(RolePermissions.Owner) &&
					!perm.contains(RolePermissions.Admin)
				);
			}) ?? [],
		[roles.data],
	);

	const incoming = connections.data?.incoming ?? [];
	const outgoing = connections.data?.outgoing ?? [];
	const pendingIncoming = incoming.filter((c) => c.status === "PENDING");
	const activeIncoming = incoming.filter((c) => c.status === "ACTIVE");

	const [showGrantDialog, setShowGrantDialog] = useState(false);
	const [showRequestDialog, setShowRequestDialog] = useState(false);
	const [approveTarget, setApproveTarget] = useState<IAppConnection | null>(
		null,
	);
	const [changeRoleTarget, setChangeRoleTarget] =
		useState<IAppConnection | null>(null);

	const [grantAppId, setGrantAppId] = useState("");
	const [grantRoleId, setGrantRoleId] = useState<string | undefined>();
	const [isGranting, setIsGranting] = useState(false);

	const [requestAppId, setRequestAppId] = useState("");
	const [requestComment, setRequestComment] = useState("");
	const [isRequesting, setIsRequesting] = useState(false);

	const refresh = useCallback(async () => {
		await invalidate(backend.teamState.getAppConnections, [appId]);
	}, [appId, backend, invalidate]);

	const invalidateGraph = useCallback(async () => {
		await invalidate(backend.teamState.getConnectionGraph, [appId, days]);
	}, [appId, backend, days, invalidate]);

	const handleCreateNote = useCallback(
		async (targetAppId: string, content: string) => {
			await backend.teamState.createProcessNote(targetAppId, content);
			await invalidateGraph();
		},
		[backend, invalidateGraph],
	);

	const handleUpdateNote = useCallback(
		async (targetAppId: string, noteId: string, content: string) => {
			await backend.teamState.updateProcessNote(targetAppId, noteId, content);
			await invalidateGraph();
		},
		[backend, invalidateGraph],
	);

	const handleDeleteNote = useCallback(
		async (targetAppId: string, noteId: string) => {
			await backend.teamState.deleteProcessNote(targetAppId, noteId);
			await invalidateGraph();
		},
		[backend, invalidateGraph],
	);

	const handleRefreshGraph = useCallback(() => {
		graph.refetch();
		cases.refetch();
	}, [graph, cases]);

	const handleGrant = useCallback(async () => {
		if (!grantAppId.trim() || !grantRoleId) {
			toast.error("Please enter an app ID and select a role");
			return;
		}

		try {
			setIsGranting(true);
			await backend.teamState.addAppConnection(
				appId,
				grantAppId.trim(),
				grantRoleId,
			);
			setShowGrantDialog(false);
			setGrantAppId("");
			setGrantRoleId(undefined);
			await refresh();
			toast.success("App access granted");
		} catch (error) {
			console.error(error);
			toast.error(
				error instanceof Error && error.message
					? error.message
					: "Failed to grant app access",
			);
		} finally {
			setIsGranting(false);
		}
	}, [appId, backend, grantAppId, grantRoleId, refresh]);

	const handleRequest = useCallback(async () => {
		if (!requestAppId.trim()) {
			toast.error("Please enter the ID of the app you want to access");
			return;
		}

		try {
			setIsRequesting(true);
			await backend.teamState.requestAppConnection(
				appId,
				requestAppId.trim(),
				requestComment.trim() || undefined,
			);
			setShowRequestDialog(false);
			setRequestAppId("");
			setRequestComment("");
			await refresh();
			toast.success("Access request sent");
		} catch (error) {
			console.error(error);
			toast.error(
				error instanceof Error && error.message
					? error.message
					: "Failed to request access",
			);
		} finally {
			setIsRequesting(false);
		}
	}, [appId, backend, requestAppId, requestComment, refresh]);

	const handleApprove = useCallback(
		async (roleId: string) => {
			if (!approveTarget) return;
			try {
				await backend.teamState.acceptAppConnection(
					appId,
					approveTarget.id,
					roleId,
				);
				setApproveTarget(null);
				await refresh();
				toast.success("App access approved");
			} catch (error) {
				console.error(error);
				toast.error(
					error instanceof Error && error.message
						? error.message
						: "Failed to approve request",
				);
			}
		},
		[appId, approveTarget, backend, refresh],
	);

	const handleReject = useCallback(
		async (connection: IAppConnection) => {
			try {
				await backend.teamState.rejectAppConnection(appId, connection.id);
				await refresh();
				toast.success("Request rejected");
			} catch (error) {
				console.error(error);
				toast.error(
					error instanceof Error && error.message
						? error.message
						: "Failed to reject request",
				);
			}
		},
		[appId, backend, refresh],
	);

	const handleChangeRole = useCallback(
		async (roleId: string) => {
			if (!changeRoleTarget) return;
			try {
				await backend.teamState.updateAppConnectionRole(
					appId,
					changeRoleTarget.id,
					roleId,
				);
				setChangeRoleTarget(null);
				await refresh();
				toast.success("Role updated");
			} catch (error) {
				console.error(error);
				toast.error(
					error instanceof Error && error.message
						? error.message
						: "Failed to update role",
				);
			}
		},
		[appId, backend, changeRoleTarget, refresh],
	);

	const handleRemove = useCallback(
		async (connection: IAppConnection) => {
			try {
				await backend.teamState.removeAppConnection(appId, connection.id);
				await refresh();
				toast.success("Connection removed");
			} catch (error) {
				console.error(error);
				toast.error(
					error instanceof Error && error.message
						? error.message
						: "Failed to remove connection",
				);
			}
		},
		[appId, backend, refresh],
	);

	return (
		<div className="space-y-8">
			<div className="inline-flex items-center gap-1 rounded-lg border border-border/60 bg-muted/40 p-1">
				<Button
					variant={view === "list" ? "secondary" : "ghost"}
					size="sm"
					onClick={() => setView("list")}
				>
					<ListIcon className="size-4" />
					List
				</Button>
				<Button
					variant={view === "graph" ? "secondary" : "ghost"}
					size="sm"
					onClick={() => setView("graph")}
				>
					<WaypointsIcon className="size-4" />
					Process graph
				</Button>
			</div>

			{view === "graph" && (
				<ProcessGraph
					appId={appId}
					data={graph.data}
					cases={cases.data?.cases}
					casesLoading={cases.isFetching}
					casesError={Boolean(cases.error)}
					isLoading={graph.isFetching}
					days={days}
					onDaysChange={setDays}
					onRefresh={handleRefreshGraph}
					onCreateNote={handleCreateNote}
					onUpdateNote={handleUpdateNote}
					onDeleteNote={handleDeleteNote}
				/>
			)}

			{view === "list" && (
				<div className="space-y-8">
					<TeamSection>
						<SectionHeading
							icon={ArrowDownLeftIcon}
							title="Apps that can reach this one"
							count={incoming.length}
							countTone={pendingIncoming.length > 0 ? "attention" : "neutral"}
							description="They can work with this app's tables, files and events under the role you grant."
							actions={
								<Button
									size="sm"
									onClick={() => setShowGrantDialog(true)}
									className={TEAM_ACTION_GRADIENT}
								>
									<PlusIcon className="size-4" />
									Grant access
								</Button>
							}
						/>

						{incoming.length === 0 ? (
							<EmptyState
								className="max-w-full"
								icons={[BlocksIcon]}
								title="No connected apps"
								description="Grant another app access to let it work with this app's data."
							/>
						) : (
							<div className="flex flex-col gap-2">
								{pendingIncoming.map((connection) => (
									<PendingRequestCard
										key={connection.id}
										connection={connection}
										onApprove={() => setApproveTarget(connection)}
										onReject={() => handleReject(connection)}
									/>
								))}
								{activeIncoming.map((connection) => (
									<ConnectionRow
										key={connection.id}
										connection={connection}
										otherAppId={connection.source_app_id}
										onChangeRole={() => setChangeRoleTarget(connection)}
										onRemove={() => handleRemove(connection)}
										removeLabel="Remove Access"
										removeDescription="This app will immediately lose access to this app's data. This action cannot be undone."
									/>
								))}
							</div>
						)}
					</TeamSection>

					<TeamSection>
						<SectionHeading
							icon={ArrowUpRightIcon}
							title="Apps this one can reach"
							count={outgoing.length}
							description="Access this app has asked for elsewhere."
							actions={
								<Button
									variant="outline"
									size="sm"
									onClick={() => setShowRequestDialog(true)}
								>
									<SendIcon className="size-4" />
									Request access
								</Button>
							}
						/>

						{outgoing.length === 0 ? (
							<EmptyState
								className="max-w-full"
								icons={[SendIcon]}
								title="No outgoing access"
								description="Request access to another app to work with its data from this app."
							/>
						) : (
							<div className="flex flex-col gap-2">
								{outgoing.map((connection) => (
									<ConnectionRow
										key={connection.id}
										connection={connection}
										otherAppId={connection.target_app_id}
										onRemove={() => handleRemove(connection)}
										removeLabel={
											connection.status === "PENDING"
												? "Cancel Request"
												: "Remove Access"
										}
										removeDescription={
											connection.status === "PENDING"
												? "The pending access request will be withdrawn."
												: "This app will lose access to the connected app's data. This action cannot be undone."
										}
									/>
								))}
							</div>
						)}
					</TeamSection>
				</div>
			)}

			{/* Grant Access Dialog */}
			<Dialog open={showGrantDialog} onOpenChange={setShowGrantDialog}>
				<DialogContent className="sm:max-w-md">
					<DialogHeader>
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
							<BlocksIcon className="h-6 w-6 text-primary" />
						</div>
						<DialogTitle className="text-center text-xl">
							Grant App Access
						</DialogTitle>
						<DialogDescription className="text-center">
							Give another app access to this app with a specific role
						</DialogDescription>
					</DialogHeader>

					<div className="space-y-4 py-4">
						<div className="space-y-2">
							<Label htmlFor="grant-app-id">App ID *</Label>
							<AppSearchPicker
								inputId="grant-app-id"
								currentAppId={appId}
								value={grantAppId}
								onChange={setGrantAppId}
								placeholder="Search apps or paste an app ID"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="grant-role">Role *</Label>
							<Select value={grantRoleId} onValueChange={setGrantRoleId}>
								<SelectTrigger>
									<SelectValue placeholder="Select a role" />
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
								The role determines what the connected app is allowed to do
							</p>
						</div>
					</div>

					<DialogFooter>
						<Button variant="outline" onClick={() => setShowGrantDialog(false)}>
							Cancel
						</Button>
						<Button
							onClick={handleGrant}
							disabled={isGranting || !grantAppId.trim() || !grantRoleId}
						>
							{isGranting ? "Granting..." : "Grant Access"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Request Access Dialog */}
			<Dialog open={showRequestDialog} onOpenChange={setShowRequestDialog}>
				<DialogContent className="sm:max-w-md">
					<DialogHeader>
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
							<SendIcon className="h-6 w-6 text-primary" />
						</div>
						<DialogTitle className="text-center text-xl">
							Request Access
						</DialogTitle>
						<DialogDescription className="text-center">
							Request access to another app in the name of this app
						</DialogDescription>
					</DialogHeader>

					<div className="space-y-4 py-4">
						<div className="space-y-2">
							<Label htmlFor="request-app-id">App ID *</Label>
							<AppSearchPicker
								inputId="request-app-id"
								currentAppId={appId}
								value={requestAppId}
								onChange={setRequestAppId}
								placeholder="Search apps or paste an app ID"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="request-comment">Message</Label>
							<Textarea
								id="request-comment"
								placeholder="Why does this app need access?"
								value={requestComment}
								onChange={(e) => setRequestComment(e.target.value)}
								className="min-h-20 resize-none"
							/>
							<p className="text-xs text-muted-foreground">
								Optional message shown to the other app&apos;s admins
							</p>
						</div>
					</div>

					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setShowRequestDialog(false)}
						>
							Cancel
						</Button>
						<Button
							onClick={handleRequest}
							disabled={isRequesting || !requestAppId.trim()}
						>
							{isRequesting ? "Sending..." : "Send Request"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<RoleSelectDialog
				open={approveTarget !== null}
				onOpenChange={(open) => {
					if (!open) setApproveTarget(null);
				}}
				title="Approve Request"
				description={`Select a role for "${connectionAppLabel(approveTarget, approveTarget?.source_app_id)}"`}
				confirmLabel="Approve"
				roles={availableRoles}
				initialRoleId={undefined}
				onConfirm={handleApprove}
			/>

			<RoleSelectDialog
				open={changeRoleTarget !== null}
				onOpenChange={(open) => {
					if (!open) setChangeRoleTarget(null);
				}}
				title="Change Role"
				description={`Select a new role for "${connectionAppLabel(changeRoleTarget, changeRoleTarget?.source_app_id)}"`}
				confirmLabel="Save Changes"
				roles={availableRoles}
				initialRoleId={changeRoleTarget?.role_id ?? undefined}
				onConfirm={handleChangeRole}
			/>
		</div>
	);
}

function connectionAppLabel(
	connection: IAppConnection | null,
	otherAppId: string | undefined,
): string {
	return connection?.app_name ?? otherAppId ?? "Unknown App";
}

interface AppSearchPickerProps {
	inputId: string;
	currentAppId: string;
	value: string;
	onChange: (appId: string) => void;
	placeholder: string;
}

function AppSearchPicker({
	inputId,
	currentAppId,
	value,
	onChange,
	placeholder,
}: Readonly<AppSearchPickerProps>) {
	const backend = useBackend();
	const [selected, setSelected] = useState<
		[IApp, IMetadata | undefined] | undefined
	>();
	const activeSelection =
		selected && selected[0].id === value ? selected : undefined;
	const search = useDebounce(value.trim(), 300);
	const searchEnabled = !activeSelection && search.length > 0;

	const ownApps = useInvoke(backend.appState.getApps, backend.appState, []);
	const storeSearch = useInvoke(
		backend.appState.searchApps,
		backend.appState,
		[undefined, search],
		searchEnabled,
	);

	const ownCandidates = useMemo(
		() => (ownApps.data ?? []).filter(([app]) => app.id !== currentAppId),
		[ownApps.data, currentAppId],
	);

	// own apps are matched locally, the store search is served by the backend
	const ownMatches = useSearch(ownCandidates, searchEnabled ? search : "", {
		fields: ["0.id", "1.name", "1.description", "1.tags"],
		boost: { "1.name": 3, "0.id": 2 },
	});

	const results = useMemo(() => {
		if (!searchEnabled) return [];
		const merged = new Map<string, [IApp, IMetadata | undefined]>();

		for (const entry of ownMatches) merged.set(entry[0].id, entry);

		for (const entry of storeSearch.data ?? []) {
			const [app] = entry;
			if (app.id !== currentAppId && !merged.has(app.id)) {
				merged.set(app.id, entry);
			}
		}

		return Array.from(merged.values());
	}, [searchEnabled, ownMatches, storeSearch.data, currentAppId]);

	const isFetching = storeSearch.isFetching || ownApps.isFetching;

	return (
		<div className="space-y-2">
			<div className="relative">
				<Input
					id={inputId}
					placeholder={placeholder}
					value={value}
					onChange={(e) => {
						setSelected(undefined);
						onChange(e.target.value);
					}}
					className="pl-10"
				/>
				<SearchIcon className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
			</div>
			<p className="text-xs text-muted-foreground">
				Search your apps by name, or paste any app ID directly to connect an app
				you don&apos;t own.
			</p>

			{activeSelection && (
				<div className="flex items-center gap-3 rounded-lg border border-primary/40 bg-primary/5 p-3">
					<AppSearchAvatar
						app={activeSelection[0]}
						metadata={activeSelection[1]}
					/>
					<div className="min-w-0 flex-1">
						<p className="truncate text-sm font-medium">
							{activeSelection[1]?.name ?? activeSelection[0].id}
						</p>
						<p className="truncate text-xs text-muted-foreground">
							{activeSelection[0].id}
						</p>
					</div>
					<CheckIcon className="h-4 w-4 shrink-0 text-primary" />
				</div>
			)}

			{searchEnabled && (
				<div className="space-y-2">
					{isFetching && results.length === 0 && (
						<div className="flex items-center justify-center gap-2 py-4 text-muted-foreground">
							<RefreshCwIcon className="h-4 w-4 animate-spin" />
							<span className="text-sm">Searching apps...</span>
						</div>
					)}

					{results.length > 0 && (
						<div className="max-h-48 space-y-2 overflow-y-auto pr-1">
							{results.map(([app, metadata]) => (
								<button
									type="button"
									key={app.id}
									onClick={() => {
										setSelected([app, metadata]);
										onChange(app.id);
									}}
									className="flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left transition-colors hover:bg-accent/50"
								>
									<AppSearchAvatar app={app} metadata={metadata} />
									<div className="min-w-0 flex-1">
										<p className="truncate text-sm font-medium">
											{metadata?.name ?? app.id}
										</p>
										<p className="truncate text-xs text-muted-foreground">
											{app.id}
										</p>
									</div>
								</button>
							))}
						</div>
					)}

					{!isFetching && results.length === 0 && (
						<p className="py-2 text-center text-xs text-muted-foreground">
							No apps found. You can still paste an app ID directly.
						</p>
					)}
				</div>
			)}
		</div>
	);
}

function AppSearchAvatar({
	app,
	metadata,
}: Readonly<{ app: IApp; metadata?: IMetadata }>) {
	const label = metadata?.name ?? app.id;

	return (
		<Avatar className="h-8 w-8">
			<AvatarImage src={metadata?.icon ?? undefined} alt={label} />
			<AvatarFallback className="bg-primary/10 text-xs text-primary">
				{label.charAt(0).toUpperCase()}
			</AvatarFallback>
		</Avatar>
	);
}

interface PendingRequestCardProps {
	connection: IAppConnection;
	onApprove: () => void;
	onReject: () => void;
}

function PendingRequestCard({
	connection,
	onApprove,
	onReject,
}: Readonly<PendingRequestCardProps>) {
	const appLabel = connectionAppLabel(connection, connection.source_app_id);

	return (
		<div className={teamRowClass({ attention: true, align: "start" })}>
			<Avatar className="size-9 shrink-0 rounded-lg">
				<AvatarImage
					src={connection.app_icon ?? undefined}
					alt={`${appLabel} icon`}
				/>
				<AvatarFallback className="rounded-lg bg-primary/10 text-primary">
					<BlocksIcon className="size-4" />
				</AvatarFallback>
			</Avatar>

			<div className="min-w-0 flex-1">
				<div className={TEAM_ROW_TITLE}>
					<span className="truncate">{appLabel}</span>
					<StatusChip tone="attention" pip>
						Wants access
					</StatusChip>
				</div>
				{connection.app_description && (
					<p className={TEAM_ROW_DESCRIPTION}>{connection.app_description}</p>
				)}
				<div className={TEAM_ROW_META}>
					<span className="flex items-center gap-1">
						<ClockIcon className="size-3.5" />
						Requested{" "}
						{new Date(connection.created_at * 1000).toLocaleDateString()}
					</span>
				</div>
				{connection.comment && <TeamRowNote>{connection.comment}</TeamRowNote>}
			</div>

			<TeamRowActions always>
				<Button size="sm" onClick={onApprove}>
					<CheckIcon className="size-3.5" />
					Approve
				</Button>
				<AlertDialog>
					<AlertDialogTrigger asChild>
						<Button size="sm" variant="outline">
							<XIcon className="size-3.5" />
							Reject
						</Button>
					</AlertDialogTrigger>
					<AlertDialogContent>
						<AlertDialogHeader>
							<AlertDialogTitle>Reject Request</AlertDialogTitle>
							<AlertDialogDescription>
								Are you sure you want to reject the access request from &quot;
								{appLabel}&quot;? The request will be deleted and the app will
								have to send a new one.
							</AlertDialogDescription>
						</AlertDialogHeader>
						<AlertDialogFooter>
							<AlertDialogCancel>Cancel</AlertDialogCancel>
							<AlertDialogAction
								onClick={onReject}
								className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							>
								Reject
							</AlertDialogAction>
						</AlertDialogFooter>
					</AlertDialogContent>
				</AlertDialog>
			</TeamRowActions>
		</div>
	);
}

interface ConnectionRowProps {
	connection: IAppConnection;
	otherAppId: string;
	onChangeRole?: () => void;
	onRemove: () => void;
	removeLabel: string;
	removeDescription: string;
}

function ConnectionRow({
	connection,
	otherAppId,
	onChangeRole,
	onRemove,
	removeLabel,
	removeDescription,
}: Readonly<ConnectionRowProps>) {
	const appLabel = connectionAppLabel(connection, otherAppId);
	const isPending = connection.status === "PENDING";
	const capabilities = useMemo(
		() => deriveConnectionCapabilities(connection.role_permissions),
		[connection.role_permissions],
	);

	return (
		<div className={teamRowClass({ align: "start" })}>
			<Avatar className="size-9 shrink-0 rounded-lg">
				<AvatarImage
					src={connection.app_icon ?? undefined}
					alt={`${appLabel} icon`}
				/>
				<AvatarFallback className="rounded-lg bg-primary/10 text-primary">
					<BlocksIcon className="size-4" />
				</AvatarFallback>
			</Avatar>

			<div className="min-w-0 flex-1">
				<div className={TEAM_ROW_TITLE}>
					<span className="truncate">{appLabel}</span>
					{isPending ? (
						<StatusChip tone="attention" pip>
							Waiting for approval
						</StatusChip>
					) : (
						<StatusChip tone="success" pip>
							Active
						</StatusChip>
					)}
					{connection.role_name && (
						<StatusChip icon={ShieldIcon}>{connection.role_name}</StatusChip>
					)}
				</div>
				{connection.app_description && (
					<p className={TEAM_ROW_DESCRIPTION}>{connection.app_description}</p>
				)}
				<div className={TEAM_ROW_META}>
					<span>
						{isPending ? "Requested " : "Connected "}
						{new Date(connection.created_at * 1000).toLocaleDateString()}
					</span>
				</div>
				<CapabilityBadges capabilities={capabilities} className="mt-1.5" />
			</div>

			<TeamRowActions>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button variant="ghost" size="icon" className="size-8">
							<MoreVerticalIcon className="size-4" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end">
						{onChangeRole && (
							<DropdownMenuItem onClick={onChangeRole}>
								<SettingsIcon className="size-4" />
								Change Role
							</DropdownMenuItem>
						)}
						<AlertDialog>
							<AlertDialogTrigger asChild>
								<DropdownMenuItem
									variant="destructive"
									onSelect={(e) => e.preventDefault()}
								>
									<Trash2Icon className="size-4" />
									{removeLabel}
								</DropdownMenuItem>
							</AlertDialogTrigger>
							<AlertDialogContent>
								<AlertDialogHeader>
									<AlertDialogTitle>{removeLabel}</AlertDialogTitle>
									<AlertDialogDescription>
										Are you sure you want to remove the connection with &quot;
										{appLabel}&quot;? {removeDescription}
									</AlertDialogDescription>
								</AlertDialogHeader>
								<AlertDialogFooter>
									<AlertDialogCancel>Cancel</AlertDialogCancel>
									<AlertDialogAction
										onClick={onRemove}
										className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
									>
										{removeLabel}
									</AlertDialogAction>
								</AlertDialogFooter>
							</AlertDialogContent>
						</AlertDialog>
					</DropdownMenuContent>
				</DropdownMenu>
			</TeamRowActions>
		</div>
	);
}

interface RoleSelectDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	title: string;
	description: string;
	confirmLabel: string;
	roles: IBackendRole[];
	initialRoleId?: string;
	onConfirm: (roleId: string) => Promise<void>;
}

function RoleSelectDialog({
	open,
	onOpenChange,
	title,
	description,
	confirmLabel,
	roles,
	initialRoleId,
	onConfirm,
}: Readonly<RoleSelectDialogProps>) {
	const [roleId, setRoleId] = useState<string | undefined>(initialRoleId);
	const [isSubmitting, setIsSubmitting] = useState(false);

	useEffect(() => {
		if (open) setRoleId(initialRoleId);
	}, [open, initialRoleId]);

	const handleConfirm = useCallback(async () => {
		if (!roleId) {
			toast.error("Please select a role");
			return;
		}

		try {
			setIsSubmitting(true);
			await onConfirm(roleId);
		} finally {
			setIsSubmitting(false);
		}
	}, [onConfirm, roleId]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
						<ShieldIcon className="h-6 w-6 text-primary" />
					</div>
					<DialogTitle className="text-center text-xl">{title}</DialogTitle>
					<DialogDescription className="text-center">
						{description}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="connection-role">Role *</Label>
						<Select value={roleId} onValueChange={setRoleId}>
							<SelectTrigger>
								<SelectValue placeholder="Select a role" />
							</SelectTrigger>
							<SelectContent>
								{roles.map((role) => (
									<SelectItem key={role.id} value={role.id}>
										{role.name}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<p className="text-xs text-muted-foreground">
							The role determines what the connected app is allowed to do
						</p>
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						Cancel
					</Button>
					<Button onClick={handleConfirm} disabled={isSubmitting || !roleId}>
						{isSubmitting ? "Saving..." : confirmLabel}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
