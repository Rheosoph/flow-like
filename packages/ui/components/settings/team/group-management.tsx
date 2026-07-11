"use client";

import {
	Boxes,
	Check,
	ChevronDown,
	Globe,
	Layers,
	Lock,
	Plus,
	Sparkles,
	Trash2,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import type { IGroup, IGroupMember, IGroupMembershipRequest } from "../../..";
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
	Badge,
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	EmptyState,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Textarea,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "../../..";

interface GroupManagementProps {
	appId: string;
}

function initials(value?: string | null): string {
	const cleaned = (value ?? "").replace(/[^A-Za-z0-9 ]/g, "").trim();
	if (!cleaned) return "?";
	const parts = cleaned.split(/\s+/);
	return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase() || "?";
}

/** Deterministic soft gradient for suites/apps without their own artwork. */
function seedGradient(seed: string): string {
	let hash = 0;
	for (let i = 0; i < seed.length; i++)
		hash = (hash * 31 + seed.charCodeAt(i)) | 0;
	const hue = ((hash % 360) + 360) % 360;
	return `linear-gradient(135deg, hsl(${hue} 62% 52%), hsl(${(hue + 42) % 360} 58% 44%))`;
}

export function GroupManagement({ appId }: Readonly<GroupManagementProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	const groups = useInvoke(backend.teamState.listGroups, backend.teamState, [
		appId,
	]);
	const requests = useInvoke(
		backend.teamState.listGroupRequests,
		backend.teamState,
		[appId],
	);
	const connections = useInvoke(
		backend.teamState.getAppConnections,
		backend.teamState,
		[appId],
	);
	const connectedApps = useMemo(() => {
		const data = connections.data;
		if (!data) return [] as { id: string; name: string }[];
		const list: { id: string; name: string }[] = [];
		for (const conn of data.incoming) {
			if (conn.status === "ACTIVE")
				list.push({
					id: conn.source_app_id,
					name: conn.app_name ?? conn.source_app_id,
				});
		}
		for (const conn of data.outgoing) {
			if (conn.status === "ACTIVE")
				list.push({
					id: conn.target_app_id,
					name: conn.app_name ?? conn.target_app_id,
				});
		}
		const seen = new Set<string>();
		return list.filter((app) => {
			if (seen.has(app.id)) return false;
			seen.add(app.id);
			return true;
		});
	}, [connections.data]);

	const [createOpen, setCreateOpen] = useState(false);
	const [busy, setBusy] = useState(false);
	const [name, setName] = useState("");
	const [useCase, setUseCase] = useState("");
	const [description, setDescription] = useState("");
	const [visibility, setVisibility] = useState("PRIVATE");

	const refresh = () => {
		invalidate(backend.teamState.listGroups, [appId]);
		invalidate(backend.teamState.listGroupRequests, [appId]);
	};

	const resetForm = () => {
		setName("");
		setUseCase("");
		setDescription("");
		setVisibility("PRIVATE");
	};

	const handleCreate = async () => {
		if (!name.trim()) return;
		setBusy(true);
		try {
			await backend.teamState.createGroup(appId, {
				name: name.trim(),
				description: description.trim() || undefined,
				use_case: useCase.trim() || undefined,
				visibility,
			});
			toast.success("Suite created");
			setCreateOpen(false);
			resetForm();
			refresh();
		} catch {
			toast.error("Could not create the suite");
		} finally {
			setBusy(false);
		}
	};

	const groupList = groups.data ?? [];
	const pendingRequests = requests.data ?? [];

	return (
		<div className="space-y-8 pb-8">
			<div className="flex items-start justify-between gap-4">
				<div>
					<h2 className="text-xl font-semibold tracking-tight flex items-center gap-2">
						<Layers className="w-5 h-5 text-primary" />
						Suites
					</h2>
					<p className="text-sm text-muted-foreground mt-1 max-w-prose">
						Curate related apps into a suite shown as one unit in the store.
						Connected apps join instantly; others receive an invite to accept.
					</p>
				</div>
				<Dialog open={createOpen} onOpenChange={setCreateOpen}>
					<Button onClick={() => setCreateOpen(true)} className="shrink-0">
						<Plus className="w-4 h-4 mr-1.5" />
						New suite
					</Button>
					<DialogContent>
						<DialogHeader>
							<DialogTitle>Create a suite</DialogTitle>
							<DialogDescription>
								Branding is borrowed from this app; you can refine it later.
							</DialogDescription>
						</DialogHeader>
						<div className="space-y-4 py-2">
							<div className="space-y-1.5">
								<Label htmlFor="suite-name">Name</Label>
								<Input
									id="suite-name"
									value={name}
									onChange={(event) => setName(event.target.value)}
									placeholder="Core Suite"
								/>
							</div>
							<div className="space-y-1.5">
								<Label htmlFor="suite-usecase">
									Suite label{" "}
									<span className="text-muted-foreground font-normal">
										(optional, shown above the app name)
									</span>
								</Label>
								<Input
									id="suite-usecase"
									value={useCase}
									onChange={(event) => setUseCase(event.target.value)}
									placeholder="Back-office platform"
								/>
							</div>
							<div className="space-y-1.5">
								<Label htmlFor="suite-desc">Description</Label>
								<Textarea
									id="suite-desc"
									value={description}
									onChange={(event) => setDescription(event.target.value)}
									placeholder="What this suite of apps does together."
									rows={3}
								/>
							</div>
							<div className="space-y-1.5">
								<Label>Visibility</Label>
								<Select value={visibility} onValueChange={setVisibility}>
									<SelectTrigger>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="PRIVATE">
											Private — only your team
										</SelectItem>
										<SelectItem value="PUBLIC">
											Public — listed in the store
										</SelectItem>
									</SelectContent>
								</Select>
							</div>
						</div>
						<DialogFooter>
							<Button variant="ghost" onClick={() => setCreateOpen(false)}>
								Cancel
							</Button>
							<Button onClick={handleCreate} disabled={busy || !name.trim()}>
								Create suite
							</Button>
						</DialogFooter>
					</DialogContent>
				</Dialog>
			</div>

			{pendingRequests.length > 0 && (
				<section className="space-y-3">
					<h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide flex items-center gap-2">
						<Sparkles className="w-4 h-4 text-primary" />
						Invitations for this app
					</h3>
					<div className="grid gap-3 sm:grid-cols-2">
						{pendingRequests.map((request) => (
							<GroupRequestCard
								key={request.membership_id}
								appId={appId}
								request={request}
								onDone={refresh}
							/>
						))}
					</div>
				</section>
			)}

			{groupList.length === 0 ? (
				<EmptyState
					title="No suites yet"
					description="Group this app with related apps into a suite that reads as one product in the store."
					icons={[Boxes, Layers, Sparkles]}
				/>
			) : (
				<div className="grid gap-4 md:grid-cols-2">
					{groupList.map((group) => (
						<GroupCard
							key={group.id}
							appId={appId}
							group={group}
							onChange={refresh}
							suggestions={connectedApps}
						/>
					))}
				</div>
			)}
		</div>
	);
}

function GroupRequestCard({
	appId,
	request,
	onDone,
}: Readonly<{
	appId: string;
	request: IGroupMembershipRequest;
	onDone: () => void;
}>) {
	const backend = useBackend();
	const [busy, setBusy] = useState(false);

	const act = async (accept: boolean) => {
		setBusy(true);
		try {
			if (accept) {
				await backend.teamState.acceptGroupRequest(
					appId,
					request.membership_id,
				);
				toast.success("Joined the suite");
			} else {
				await backend.teamState.declineGroupRequest(
					appId,
					request.membership_id,
				);
				toast.success("Invitation declined");
			}
			onDone();
		} catch {
			toast.error("Could not update the invitation");
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="rounded-xl border bg-card p-4 flex items-center gap-3">
			<Avatar className="h-10 w-10 rounded-lg">
				{request.group_icon ? (
					<AvatarImage src={request.group_icon} alt="" />
				) : null}
				<AvatarFallback
					className="rounded-lg text-white text-xs font-bold"
					style={{ backgroundImage: seedGradient(request.group_id) }}
				>
					{initials(request.group_name)}
				</AvatarFallback>
			</Avatar>
			<div className="min-w-0 flex-1">
				<p className="text-sm font-medium truncate">
					{request.group_name ?? "A suite"}
				</p>
				<p className="text-xs text-muted-foreground">
					wants to feature your app
				</p>
			</div>
			<div className="flex items-center gap-1.5">
				<Button
					size="sm"
					variant="ghost"
					disabled={busy}
					onClick={() => act(false)}
				>
					<X className="w-4 h-4" />
				</Button>
				<Button size="sm" disabled={busy} onClick={() => act(true)}>
					<Check className="w-4 h-4 mr-1" />
					Accept
				</Button>
			</div>
		</div>
	);
}

function GroupCard({
	appId,
	group,
	onChange,
	suggestions,
}: Readonly<{
	appId: string;
	group: IGroup;
	onChange: () => void;
	suggestions: { id: string; name: string }[];
}>) {
	const backend = useBackend();
	const [expanded, setExpanded] = useState(false);
	const [memberInput, setMemberInput] = useState("");
	const [busy, setBusy] = useState(false);

	const isOwner = group.owner_app_id === appId;
	const isPublic = group.visibility === "PUBLIC";
	const label = group.use_case || group.name || "Untitled suite";
	const memberIds = new Set(group.members.map((member) => member.app_id));
	const addable = suggestions.filter((app) => !memberIds.has(app.id));

	const addMember = async (targetId?: string) => {
		const target = (targetId ?? memberInput).trim();
		if (!target) return;
		setBusy(true);
		try {
			await backend.teamState.addGroupMember(appId, group.id, target);
			toast.success("App added to the suite");
			setMemberInput("");
			onChange();
		} catch {
			toast.error("Could not add the app");
		} finally {
			setBusy(false);
		}
	};

	const removeMember = async (memberAppId: string) => {
		setBusy(true);
		try {
			await backend.teamState.removeGroupMember(appId, group.id, memberAppId);
			toast.success("App removed from the suite");
			onChange();
		} catch {
			toast.error("Could not remove the app");
		} finally {
			setBusy(false);
		}
	};

	const deleteGroup = async () => {
		setBusy(true);
		try {
			await backend.teamState.deleteGroup(appId, group.id);
			toast.success("Suite deleted");
			onChange();
		} catch {
			toast.error("Could not delete the suite");
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="rounded-xl border bg-card overflow-hidden flex flex-col">
			<div
				className="h-16 relative"
				style={{
					backgroundImage: group.banner ? undefined : seedGradient(group.id),
				}}
			>
				{group.banner && (
					// eslint-disable-next-line @next/next/no-img-element
					<img
						src={group.banner}
						alt=""
						className="absolute inset-0 h-full w-full object-cover"
					/>
				)}
			</div>
			<div className="px-4 pb-4 -mt-6 flex flex-col gap-3 flex-1">
				<div className="flex items-end justify-between gap-3">
					<Avatar className="h-12 w-12 rounded-xl ring-4 ring-card">
						{group.icon ? <AvatarImage src={group.icon} alt="" /> : null}
						<AvatarFallback
							className="rounded-xl text-white font-bold"
							style={{ backgroundImage: seedGradient(group.id) }}
						>
							{initials(group.name)}
						</AvatarFallback>
					</Avatar>
					<Badge
						variant="secondary"
						className="mb-1 gap-1 text-[11px] font-medium"
					>
						{isPublic ? (
							<Globe className="w-3 h-3" />
						) : (
							<Lock className="w-3 h-3" />
						)}
						{isPublic ? "Public" : "Private"}
					</Badge>
				</div>

				<div>
					<p className="font-semibold leading-tight">{label}</p>
					{group.use_case && group.name && (
						<p className="text-xs text-muted-foreground mt-0.5">{group.name}</p>
					)}
					{group.description && (
						<p className="text-xs text-muted-foreground mt-1 line-clamp-2">
							{group.description}
						</p>
					)}
				</div>

				<div className="flex items-center justify-between mt-auto pt-1">
					<div className="flex items-center -space-x-2">
						{group.members.slice(0, 5).map((member) => (
							<Avatar
								key={member.id}
								className="h-6 w-6 rounded-md ring-2 ring-card"
							>
								{member.app_icon ? (
									<AvatarImage src={member.app_icon} alt="" />
								) : null}
								<AvatarFallback
									className="rounded-md text-white text-[9px] font-bold"
									style={{ backgroundImage: seedGradient(member.app_id) }}
								>
									{initials(member.app_name)}
								</AvatarFallback>
							</Avatar>
						))}
						<span className="pl-3 text-xs text-muted-foreground font-medium">
							{group.member_count} app{group.member_count === 1 ? "" : "s"}
						</span>
					</div>
					<button
						type="button"
						onClick={() => setExpanded((value) => !value)}
						className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 transition-colors"
					>
						Manage
						<ChevronDown
							className={`w-3.5 h-3.5 transition-transform ${expanded ? "rotate-180" : ""}`}
						/>
					</button>
				</div>

				{expanded && (
					<div className="mt-2 pt-3 border-t space-y-3">
						<div className="space-y-1.5">
							{group.members.map((member) => (
								<MemberRow
									key={member.id}
									member={member}
									canRemove={isOwner && member.kind !== "PRIMARY"}
									disabled={busy}
									onRemove={() => removeMember(member.app_id)}
								/>
							))}
						</div>

						{isOwner && (
							<div className="space-y-2">
								{addable.length > 0 && (
									<div className="flex flex-wrap gap-1.5">
										<span className="text-[11px] text-muted-foreground w-full">
											Connected apps — join instantly:
										</span>
										{addable.map((app) => (
											<button
												key={app.id}
												type="button"
												disabled={busy}
												onClick={() => addMember(app.id)}
												className="inline-flex items-center gap-1 rounded-full border bg-background px-2.5 py-1 text-[11px] transition-colors hover:border-primary/50 hover:bg-primary/5 disabled:opacity-50"
											>
												<Plus className="w-3 h-3" />
												{app.name}
											</button>
										))}
									</div>
								)}
								<div className="flex items-center gap-2">
									<Input
										value={memberInput}
										onChange={(event) => setMemberInput(event.target.value)}
										placeholder="App ID to add…"
										className="h-8 text-xs"
									/>
									<Button
										size="sm"
										variant="secondary"
										disabled={busy || !memberInput.trim()}
										onClick={() => addMember()}
									>
										<Plus className="w-3.5 h-3.5" />
									</Button>
								</div>
							</div>
						)}

						{isOwner && (
							<AlertDialog>
								<AlertDialogTrigger asChild>
									<Button
										variant="ghost"
										size="sm"
										className="text-destructive hover:text-destructive w-full justify-start"
									>
										<Trash2 className="w-3.5 h-3.5 mr-1.5" />
										Delete suite
									</Button>
								</AlertDialogTrigger>
								<AlertDialogContent>
									<AlertDialogHeader>
										<AlertDialogTitle>Delete this suite?</AlertDialogTitle>
										<AlertDialogDescription>
											The suite and its curation are removed. The member apps
											themselves are not affected.
										</AlertDialogDescription>
									</AlertDialogHeader>
									<AlertDialogFooter>
										<AlertDialogCancel>Cancel</AlertDialogCancel>
										<AlertDialogAction onClick={deleteGroup}>
											Delete
										</AlertDialogAction>
									</AlertDialogFooter>
								</AlertDialogContent>
							</AlertDialog>
						)}
					</div>
				)}
			</div>
		</div>
	);
}

function MemberRow({
	member,
	canRemove,
	disabled,
	onRemove,
}: Readonly<{
	member: IGroupMember;
	canRemove: boolean;
	disabled: boolean;
	onRemove: () => void;
}>) {
	const isPrimary = member.kind === "PRIMARY";
	const isPending = member.status === "PENDING";
	return (
		<div className="flex items-center gap-2.5">
			<Avatar className="h-7 w-7 rounded-md">
				{member.app_icon ? <AvatarImage src={member.app_icon} alt="" /> : null}
				<AvatarFallback
					className="rounded-md text-white text-[10px] font-bold"
					style={{ backgroundImage: seedGradient(member.app_id) }}
				>
					{initials(member.app_name)}
				</AvatarFallback>
			</Avatar>
			<span className="text-sm truncate flex-1">
				{member.app_name ?? member.app_id}
			</span>
			{isPrimary && (
				<Badge variant="outline" className="text-[10px]">
					Anchor
				</Badge>
			)}
			{isPending && (
				<Badge variant="secondary" className="text-[10px]">
					Pending
				</Badge>
			)}
			{canRemove && (
				<Button
					size="icon"
					variant="ghost"
					className="h-6 w-6"
					disabled={disabled}
					onClick={onRemove}
				>
					<X className="w-3.5 h-3.5" />
				</Button>
			)}
		</div>
	);
}
