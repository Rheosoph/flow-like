"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Check,
	Layers,
	Plus,
	Settings2,
	Sparkles,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import type { IGroup, IGroupMembershipRequest } from "../../..";
import {
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
	Textarea,
	initials,
	seedGradient,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "../../..";
import {
	VISIBILITY_META,
	fromWireVisibility,
} from "../visibility-status/visibility-meta";
import { GroupConsole } from "./group-console";

interface GroupManagementProps {
	appId: string;
}

export function GroupManagement({ appId }: Readonly<GroupManagementProps>) {
	const { t } = useTranslation("settings");
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
		for (const conn of data.incoming ?? []) {
			if (conn.status === "ACTIVE")
				list.push({
					id: conn.source_app_id,
					name: conn.app_name ?? conn.source_app_id,
				});
		}
		for (const conn of data.outgoing ?? []) {
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

	const refresh = () => {
		invalidate(backend.teamState.listGroups, [appId]);
		invalidate(backend.teamState.listGroupRequests, [appId]);
	};

	const resetForm = () => {
		setName("");
		setUseCase("");
		setDescription("");
	};

	const handleCreate = async () => {
		if (!name.trim()) return;
		setBusy(true);
		try {
			await backend.teamState.createGroup(appId, {
				name: name.trim(),
				description: description.trim() || undefined,
				use_case: useCase.trim() || undefined,
			});
			toast.success("Suite created");
			setCreateOpen(false);
			resetForm();
			refresh();
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t("couldNotCreateTheSuite", "Could not create the suite"),
			);
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
						{t("suites", "Suites")}
					</h2>
					<p className="text-sm text-muted-foreground mt-1 max-w-prose">
						{t(
							"curateRelatedAppsIntoASuiteShownAsOneUnitInTheStoreConnectedAppsJoinInstantlyOthersReceiveAnInviteToAccept",
							"Curate related apps into a suite shown as one unit in the store. Connected apps join instantly; others receive an invite to accept.",
						)}
					</p>
				</div>
				<Dialog open={createOpen} onOpenChange={setCreateOpen}>
					<Button onClick={() => setCreateOpen(true)} className="shrink-0">
						<Plus className="w-4 h-4 mr-1.5" />
						{t("newSuite", "New suite")}
					</Button>
					<DialogContent>
						<DialogHeader>
							<DialogTitle>{t("createASuite", "Create a suite")}</DialogTitle>
							<DialogDescription>
								{`Suites start private. Add artwork, apps and publish it from the suite console.`}
							</DialogDescription>
						</DialogHeader>
						<div className="space-y-4 py-2">
							<div className="space-y-1.5">
								<Label htmlFor="suite-name">Name</Label>
								<Input
									id="suite-name"
									value={name}
									onChange={(event) => setName(event.target.value)}
									placeholder={t("coreSuite", "Core Suite")}
								/>
							</div>
							<div className="space-y-1.5">
								<Label htmlFor="suite-usecase">
									{t("suiteLabel", "Suite label")}{" "}
									<span className="text-muted-foreground font-normal">
										{t(
											"optionalShownAboveTheAppName",
											"(optional, shown above the app name)",
										)}
									</span>
								</Label>
								<Input
									id="suite-usecase"
									value={useCase}
									onChange={(event) => setUseCase(event.target.value)}
									placeholder={t("backofficePlatform", "Back-office platform")}
								/>
							</div>
							<div className="space-y-1.5">
								<Label htmlFor="suite-desc">
									{t("description", "Description")}
								</Label>
								<Textarea
									id="suite-desc"
									value={description}
									onChange={(event) => setDescription(event.target.value)}
									placeholder={t(
										"whatThisSuiteOfAppsDoesTogether",
										"What this suite of apps does together.",
									)}
									rows={3}
								/>
							</div>
						</div>
						<DialogFooter>
							<Button variant="ghost" onClick={() => setCreateOpen(false)}>
								{t("cancel", "Cancel")}
							</Button>
							<Button onClick={handleCreate} disabled={busy || !name.trim()}>
								{t("createSuite", "Create suite")}
							</Button>
						</DialogFooter>
					</DialogContent>
				</Dialog>
			</div>

			{pendingRequests.length > 0 && (
				<section className="space-y-3">
					<h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide flex items-center gap-2">
						<Sparkles className="w-4 h-4 text-primary" />
						{t("invitationsForThisApp", "Invitations for this app")}
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
					title={t("noSuitesYet", "No suites yet")}
					description={t(
						"groupThisAppWithRelatedAppsIntoASuiteThatReadsAsOneProductInTheStore",
						"Group this app with related apps into a suite that reads as one product in the store.",
					)}
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
	const { t } = useTranslation("settings");
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
					{request.group_name ?? t("aSuite", "A suite")}
				</p>
				<p className="text-xs text-muted-foreground">
					{t("wantsToFeatureYourApp", "wants to feature your app")}
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
					{t("accept", "Accept")}
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
	const { t } = useTranslation("settings");
	const [consoleOpen, setConsoleOpen] = useState(false);
	const label = group.use_case || group.name || "Untitled suite";
	const meta = VISIBILITY_META[fromWireVisibility(group.visibility)];

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
						className="mb-1 gap-1.5 text-[11px] font-medium"
					>
						<span className={`w-2 h-2 rounded-full ${meta.color}`} />
						{meta.title}
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
							{t("countApps", {
								defaultValue_one: "{{count}} app",
								defaultValue_other: "{{count}} apps",
								count: group.member_count,
							})}
						</span>
					</div>
					<Button
						size="sm"
						variant="outline"
						onClick={() => setConsoleOpen(true)}
					>
						<Settings2 className="w-3.5 h-3.5 mr-1.5" />
						{t("manage", "Manage")}
					</Button>
				</div>
			</div>

			<GroupConsole
				appId={appId}
				group={group}
				open={consoleOpen}
				onOpenChange={setConsoleOpen}
				onChange={onChange}
				suggestions={suggestions}
			/>
		</div>
	);
}
