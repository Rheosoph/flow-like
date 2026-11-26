"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2Icon, SearchIcon } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { toast } from "sonner";
import { useProjectUserSearch } from "../../../hooks/use-project-user-search";
import { isTeamAdmin, teamPath } from "../../../lib/bot-teams/org-model";
import { userDisplayName, userSecondaryLabel } from "../../../lib/user-display";
import type {
	IOrgBot,
	ITeam,
	TeamRole,
} from "../../../state/backend-state/bot-team-state";
import { IdentityAvatar } from "../../identity/identity-avatar";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";
import { useOrgChart } from "./org-chart-context";
import {
	dropAction,
	isFlux,
	newTeamParents,
	orgPeopleOutside,
	toOrgBot,
} from "./org-chart-model";
import { OrgBotAvatar, PersonName } from "./org-chart-parts";

const TOP_LEVEL = "__top__";

export function NewTeamDialog({
	parentId,
	onClose,
}: Readonly<{ parentId: string | null; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	const { org, me, actions, select } = useOrgChart();
	const [name, setName] = useState("");
	const [purpose, setPurpose] = useState("");
	const [parent, setParent] = useState(parentId ?? TOP_LEVEL);
	const parents = newTeamParents(org, me);
	const submit = () => {
		actions.createTeam.mutate(
			{
				name: name.trim(),
				purpose: purpose.trim() || null,
				parent_id: parent === TOP_LEVEL ? null : parent,
			},
			{
				onSuccess: (team) => {
					const created = team as ITeam | undefined;
					if (created?.id) select({ kind: "team", id: created.id });
					onClose();
				},
				onError: (error) => toast.error(error.message),
			},
		);
	};
	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>{t("teams.org.newTeam", "New team")}</DialogTitle>
					<DialogDescription>
						{t(
							"teams.org.newTeamHint",
							"You become its admin. Add people and bots afterwards.",
						)}
					</DialogDescription>
				</DialogHeader>
				<form
					className="flex flex-col gap-4"
					onSubmit={(event) => {
						event.preventDefault();
						if (name.trim()) submit();
					}}
				>
					<div className="flex flex-col gap-1.5">
						<Label htmlFor="org-new-team-name">
							{t("teams.org.teamName", "Team name")}
						</Label>
						<Input
							id="org-new-team-name"
							value={name}
							onChange={(event) => setName(event.target.value)}
						/>
					</div>
					<div className="flex flex-col gap-1.5">
						<Label htmlFor="org-new-team-purpose">
							{t("teams.org.purpose", "Purpose")}
						</Label>
						<Textarea
							id="org-new-team-purpose"
							rows={3}
							value={purpose}
							placeholder={t(
								"teams.org.purposePlaceholder",
								"What does this team do?",
							)}
							onChange={(event) => setPurpose(event.target.value)}
						/>
					</div>
					<div className="flex flex-col gap-1.5">
						<Label>{t("teams.org.partOf", "Part of")}</Label>
						<Select value={parent} onValueChange={setParent}>
							<SelectTrigger aria-label={t("teams.org.partOf", "Part of")}>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={TOP_LEVEL}>
									{t("teams.org.topLevel", "Top level")}
								</SelectItem>
								{parents.map((team) => (
									<SelectItem key={team.id} value={team.id}>
										{team.name}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>
					<DialogFooter>
						<Button type="button" variant="ghost" onClick={onClose}>
							{t("teams.org.cancel", "Cancel")}
						</Button>
						<Button
							type="submit"
							disabled={!name.trim() || actions.createTeam.isPending}
						>
							{t("teams.org.createTeam", "Create team")}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function PersonInviteRow({
	userId,
	name,
	detail,
	already,
	disabled,
	onInvite,
}: Readonly<{
	userId: string;
	name: ReactNode;
	detail?: string;
	already: boolean;
	disabled: boolean;
	onInvite: () => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<li className="flex items-center gap-3 px-1 py-1">
			<IdentityAvatar subject={userId} size={32} />
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate text-sm font-medium">{name}</span>
				{detail && (
					<span className="truncate text-xs text-muted-foreground">
						{detail}
					</span>
				)}
			</span>
			<Button
				size="sm"
				variant="outline"
				disabled={already || disabled}
				onClick={onInvite}
			>
				{already
					? t("teams.org.alreadyIn", "In the team")
					: t("teams.org.invite", "Invite")}
			</Button>
		</li>
	);
}

function EmptyLine({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<li className="px-1 py-4 text-center text-xs text-muted-foreground">
			{children}
		</li>
	);
}

export function AddPeopleDialog({
	team,
	onClose,
}: Readonly<{ team: ITeam; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	const { org, me, actions } = useOrgChart();
	const [query, setQuery] = useState("");
	const [role, setRole] = useState<TeamRole>("member");
	const search = useProjectUserSearch(undefined, query, true);
	const present = new Set(team.people.map((person) => person.user_id));
	const rows = search.results.filter(({ user }) => !user.bot && user.id !== me);
	const suggestions = useMemo(
		() => orgPeopleOutside(org, team, me),
		[org, team, me],
	);
	const orgName = teamPath(org, team.id)[0]?.name ?? team.name;
	const searching = query.trim() !== "";
	const invite = (userId: string) =>
		actions.invitePeople.mutate(
			{ teamId: team.id, userIds: [userId], role },
			{
				onSuccess: () =>
					toast.success(t("teams.org.toast.personInvited", "Invitation sent")),
				onError: (error) => toast.error(error.message),
			},
		);
	const busy = actions.invitePeople.isPending;
	const searchEmpty = () => {
		if (search.isSearchingDirectory) return null;
		if (search.directoryError)
			return (
				<EmptyLine>
					{t("teams.org.searchFailed", "Searching people failed.")}
				</EmptyLine>
			);
		return (
			<EmptyLine>
				{search.canSearchDirectory
					? t("teams.org.noPeopleMatch", "Nobody found for “{{query}}”.", {
							query: query.trim(),
						})
					: t("teams.org.noPeople", "Type a name or email to find people.")}
			</EmptyLine>
		);
	};
	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>
						{t("teams.org.invitePeopleTo", "Invite people to {{team}}", {
							team: team.name,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"teams.org.invitePeopleHint",
							"They see the whole org once they accept.",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="flex items-center gap-2">
					<label className="flex h-9 flex-1 items-center gap-2 rounded-lg border bg-card px-2.5 text-muted-foreground focus-within:border-primary">
						<SearchIcon className="size-4 shrink-0" aria-hidden />
						<input
							type="search"
							value={query}
							onChange={(event) => setQuery(event.target.value)}
							aria-label={t("teams.org.searchPeople", "Search people")}
							placeholder={t("teams.org.searchPeople", "Search people")}
							className="h-8 min-w-0 flex-1 appearance-none bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
						/>
						{search.isSearchingDirectory && (
							<Loader2Icon className="size-4 animate-spin" aria-hidden />
						)}
					</label>
					<Select
						value={role}
						onValueChange={(value) => setRole(value as TeamRole)}
					>
						<SelectTrigger
							className="w-28"
							aria-label={t("teams.org.role", "Role")}
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="member">
								{t("teams.org.roleMember", "Member")}
							</SelectItem>
							<SelectItem value="admin">
								{t("teams.org.roleAdmin", "Admin")}
							</SelectItem>
						</SelectContent>
					</Select>
				</div>
				<ul className="flex max-h-72 flex-col gap-1 overflow-y-auto">
					{searching ? (
						<>
							{rows.map(({ user }) => (
								<PersonInviteRow
									key={user.id}
									userId={user.id}
									name={userDisplayName(user)}
									detail={userSecondaryLabel(user)}
									already={present.has(user.id)}
									disabled={busy}
									onInvite={() => invite(user.id)}
								/>
							))}
							{rows.length === 0 && searchEmpty()}
						</>
					) : suggestions.length > 0 ? (
						<>
							<li className="px-1 pt-1 text-xs font-medium text-muted-foreground">
								{t("teams.org.inOrg", "Already in {{org}}", { org: orgName })}
							</li>
							{suggestions.map((person) => (
								<PersonInviteRow
									key={person.userId}
									userId={person.userId}
									name={<PersonName userId={person.userId} />}
									detail={person.teams.join(" · ")}
									already={false}
									disabled={busy}
									onInvite={() => invite(person.userId)}
								/>
							))}
						</>
					) : (
						<EmptyLine>
							{t("teams.org.noPeople", "Type a name or email to find people.")}
						</EmptyLine>
					)}
				</ul>
				<DialogFooter>
					<Button variant="ghost" onClick={onClose}>
						{t("teams.org.done", "Done")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function AddBotRow({
	team,
	bot,
	onDone,
}: Readonly<{ team: ITeam; bot: IOrgBot; onDone: () => void }>) {
	const { t } = useTranslation("bots");
	const { me, nameOf, placeBot, actions } = useOrgChart();
	const action = dropAction(team, bot, me);
	return (
		<li className="flex items-center gap-3 px-1 py-1">
			<OrgBotAvatar bot={bot} size={28} />
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate text-sm font-medium">{nameOf(bot)}</span>
				{bot.owner_id !== me && (
					<span className="truncate text-xs text-muted-foreground">
						<PersonName userId={bot.owner_id} />
					</span>
				)}
			</span>
			<Button
				size="sm"
				variant="outline"
				disabled={!action || actions.addBot.isPending}
				onClick={() => {
					placeBot(team, bot);
					onDone();
				}}
			>
				{action === "invite"
					? t("teams.org.inviteBot", "Invite")
					: t("teams.org.addNow", "Add")}
			</Button>
		</li>
	);
}

export function AddBotDialog({
	team,
	onClose,
}: Readonly<{ team: ITeam; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	const { org, me, ownBots } = useOrgChart();
	const inTeam = new Set(team.bots.map((entry) => entry.bot_id));
	const mine = ownBots
		.filter((bot) => !isFlux(bot) && !inTeam.has(bot.id))
		.map(toOrgBot);
	const others = isTeamAdmin(team, me)
		? org.bots.filter(
				(bot) => bot.owner_id !== me && !isFlux(bot) && !inTeam.has(bot.id),
			)
		: [];
	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>
						{t("teams.org.addBotTo", "Add a bot to {{team}}", {
							team: team.name,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"teams.org.addBotHint",
							"Your bots join now and leave their current team. Another person's bot waits until its owner accepts.",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="flex max-h-80 flex-col gap-3 overflow-y-auto">
					<section className="flex flex-col gap-1">
						<h3 className="text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
							{t("teams.org.yourBots", "Your bots")}
						</h3>
						<ul>
							{mine.map((bot) => (
								<AddBotRow
									key={bot.id}
									team={team}
									bot={bot}
									onDone={onClose}
								/>
							))}
						</ul>
						{mine.length === 0 && (
							<p className="text-xs text-muted-foreground">
								{t(
									"teams.org.noOwnBots",
									"All your bots are already in this team.",
								)}
							</p>
						)}
					</section>
					{others.length > 0 && (
						<section className="flex flex-col gap-1">
							<h3 className="text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
								{t("teams.org.othersBots", "Other people's bots")}
							</h3>
							<ul>
								{others.map((bot) => (
									<AddBotRow
										key={bot.id}
										team={team}
										bot={bot}
										onDone={onClose}
									/>
								))}
							</ul>
						</section>
					)}
				</div>
				<DialogFooter>
					<Button variant="ghost" onClick={onClose}>
						{t("teams.org.done", "Done")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function OrgChartDialogs() {
	const { org, dialog, setDialog } = useOrgChart();
	const close = () => setDialog(null);
	if (!dialog) return null;
	if (dialog.kind === "newTeam") {
		return <NewTeamDialog parentId={dialog.parentId} onClose={close} />;
	}
	const team = org.teams.find((entry) => entry.id === dialog.teamId);
	if (!team) return null;
	return dialog.kind === "addPeople" ? (
		<AddPeopleDialog team={team} onClose={close} />
	) : (
		<AddBotDialog team={team} onClose={close} />
	);
}
