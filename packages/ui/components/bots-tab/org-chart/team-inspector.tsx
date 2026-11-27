"use client";

import { useTranslation } from "@flow-like/locales";
import {
	LockIcon,
	LogOutIcon,
	PlusIcon,
	Trash2Icon,
	UserPlusIcon,
	UsersIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useUserIdentity } from "../../../hooks/use-user-lookup";
import {
	activeBots,
	classificationOf,
	isTeamAdmin,
	isTeamMember,
	leadOfferFor,
	rootTeamId,
	teamById,
} from "../../../lib/bot-teams/org-model";
import {
	DATA_CLASSIFICATIONS,
	type DataClassification,
} from "../../../lib/data-classification";
import type {
	ITeam,
	IUpdateTeamRequest,
	TeamRole,
} from "../../../state/backend-state/bot-team-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Switch } from "../../ui/switch";
import { Textarea } from "../../ui/textarea";
import { TeamAppsSection } from "./app-deck";
import { InspectorSection } from "./inspector-parts";
import { TeamIoSection } from "./io-inspector";
import { useOrgChart } from "./org-chart-context";
import {
	canTakeOut,
	isWaitingForConsent,
	partOfOptions,
	teamCounts,
} from "./org-chart-model";
import { OrgBotAvatar, PersonFace, PersonName } from "./org-chart-parts";
import { TeamIcon } from "./team-icon-picker";

const NONE = "__none__";

function useTeamUpdate(team: ITeam) {
	const { actions } = useOrgChart();
	return (request: IUpdateTeamRequest) =>
		actions.updateTeam.mutate(
			{ teamId: team.id, request },
			{ onError: (error) => toast.error(error.message) },
		);
}

function NameAndPurpose({
	team,
	admin,
}: Readonly<{ team: ITeam; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const update = useTeamUpdate(team);
	const [name, setName] = useState(team.name);
	const [purpose, setPurpose] = useState(team.purpose ?? "");
	useEffect(() => setName(team.name), [team.name]);
	useEffect(() => setPurpose(team.purpose ?? ""), [team.purpose]);
	const commitName = () => {
		const next = name.trim();
		if (!next) return setName(team.name);
		if (next !== team.name) update({ name: next });
	};
	const purposeDirty = purpose.trim() !== (team.purpose ?? "").trim();
	return (
		<>
			{admin && (
				<Input
					value={name}
					aria-label={t("teams.org.teamName", "Team name")}
					onChange={(event) => setName(event.target.value)}
					onBlur={commitName}
					onKeyDown={(event) =>
						event.key === "Enter" && event.currentTarget.blur()
					}
				/>
			)}
			<InspectorSection
				title={t("teams.org.purpose", "Purpose")}
				hint={t("teams.org.purposeHint", "Every bot in the team reads this.")}
			>
				{admin ? (
					<>
						<Textarea
							value={purpose}
							rows={3}
							aria-label={t("teams.org.purpose", "Purpose")}
							placeholder={t(
								"teams.org.purposePlaceholder",
								"What does this team do?",
							)}
							onChange={(event) => setPurpose(event.target.value)}
						/>
						{purposeDirty && (
							<Button
								size="sm"
								className="self-start"
								onClick={() => update({ purpose: purpose.trim() || null })}
							>
								{t("teams.org.savePurpose", "Save purpose")}
							</Button>
						)}
					</>
				) : (
					<p className="text-sm">
						{team.purpose || (
							<span className="text-muted-foreground">
								{t("teams.org.noPurpose", "No purpose written yet.")}
							</span>
						)}
					</p>
				)}
			</InspectorSection>
		</>
	);
}

function OfferWaiting({
	ownerId,
	botName,
}: Readonly<{ ownerId: string; botName: string }>) {
	const { t } = useTranslation("bots");
	const owner = useUserIdentity(ownerId);
	return (
		<span className="text-warning">
			{t("teams.reach.offerWaiting", "Waiting for {{owner}} to accept", {
				owner: owner.label,
			})}
			{botName && <span className="text-muted-foreground"> ({botName})</span>}
		</span>
	);
}

function LeadOffer({
	team,
	update,
}: Readonly<{
	team: ITeam;
	update: (request: IUpdateTeamRequest) => void;
}>) {
	const { t } = useTranslation("bots");
	const { botOf, nameOf } = useOrgChart();
	const [confirming, setConfirming] = useState(false);
	const offer = leadOfferFor(team);
	if (!offer) return null;
	const bot = botOf(offer.bot_id);
	return (
		<div className="flex flex-col gap-1 text-xs">
			<OfferWaiting ownerId={offer.owner_id} botName={bot ? nameOf(bot) : ""} />
			<Button
				variant="ghost"
				size="sm"
				className="h-6 self-start px-2 text-xs"
				onClick={() => setConfirming(true)}
			>
				{t("teams.reach.withdrawOffer", "Withdraw")}
			</Button>
			<AlertDialog open={confirming} onOpenChange={setConfirming}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("teams.reach.withdrawTitle", "Withdraw the lead request?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"teams.reach.withdrawBody",
								"This clears the request and the current lead: {{team}} has no lead until you pick one.",
								{ team: team.name },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("teams.org.cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction onClick={() => update({ lead_bot_id: null })}>
							{t("teams.reach.withdrawConfirm", "Withdraw")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

function Placement({ team, admin }: Readonly<{ team: ITeam; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const { org, me, botOf, nameOf } = useOrgChart();
	const update = useTeamUpdate(team);
	const parents = partOfOptions(org, team, me);
	const current = team.parent_id
		? org.teams.find((entry) => entry.id === team.parent_id)
		: undefined;
	const parentOptions =
		current && !parents.some((entry) => entry.id === current.id)
			? [...parents, current]
			: parents;
	const leads = activeBots(team).flatMap((entry) => {
		const bot = botOf(entry.bot_id);
		return bot ? [bot] : [];
	});
	return (
		<div className="grid grid-cols-2 gap-3">
			<InspectorSection
				title={t("teams.org.leadLabel", "Lead")}
				hint={t(
					"teams.reach.leadPickerHint",
					"Another person's bot leads only after its owner accepts.",
				)}
			>
				<Select
					disabled={!admin}
					value={team.lead_bot_id ?? NONE}
					onValueChange={(value) =>
						update({ lead_bot_id: value === NONE ? null : value })
					}
				>
					<SelectTrigger aria-label={t("teams.org.leadLabel", "Lead")}>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value={NONE}>
							{t("teams.org.noLeadOption", "No lead")}
						</SelectItem>
						{leads.map((bot) => (
							<SelectItem key={bot.id} value={bot.id}>
								{nameOf(bot)}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				{admin && <LeadOffer team={team} update={update} />}
			</InspectorSection>
			<InspectorSection title={t("teams.org.partOf", "Part of")}>
				<Select
					disabled={!admin}
					value={team.parent_id ?? NONE}
					onValueChange={(value) =>
						update({ parent_id: value === NONE ? null : value })
					}
				>
					<SelectTrigger aria-label={t("teams.org.partOf", "Part of")}>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value={NONE}>
							{t("teams.org.topLevel", "Top level")}
						</SelectItem>
						{parentOptions.map((entry) => (
							<SelectItem key={entry.id} value={entry.id}>
								{entry.name}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</InspectorSection>
		</div>
	);
}

function ClassificationSelect({
	team,
	userId,
}: Readonly<{ team: ITeam; userId: string }>) {
	const { t } = useTranslation("bots");
	const { org, actions } = useOrgChart();
	const explicit = org.classifications?.some(
		(entry) => entry.org_id === team.id && entry.user_id === userId,
	);
	const labels: Record<DataClassification, string> = {
		public: t("teams.reach.level.public", "Public"),
		internal: explicit
			? t("teams.reach.level.internal", "Internal")
			: t("teams.reach.level.internalDefault", "Internal (default)"),
		confidential: t("teams.reach.level.confidential", "Confidential"),
		restricted: t("teams.reach.level.restricted", "Restricted"),
	};
	return (
		<Select
			value={classificationOf(org, team.id, userId)}
			onValueChange={(value) =>
				actions.setClassification.mutate(
					{
						orgId: team.id,
						userId,
						classification: value as DataClassification,
					},
					{ onError: (error) => toast.error(error.message) },
				)
			}
		>
			<SelectTrigger
				size="sm"
				className="h-6 w-40 text-[11px]"
				aria-label={t("teams.reach.classification", "Classification")}
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{DATA_CLASSIFICATIONS.map((level) => (
					<SelectItem key={level} value={level}>
						{labels[level]}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

function People({ team, admin }: Readonly<{ team: ITeam; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const { me, actions, setDialog } = useOrgChart();
	const fail = { onError: (error: Error) => toast.error(error.message) };
	return (
		<InspectorSection
			title={t("teams.org.people", "People")}
			action={
				admin && (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 text-xs"
						onClick={() => setDialog({ kind: "addPeople", teamId: team.id })}
					>
						<UserPlusIcon aria-hidden />
						{t("teams.org.invitePeople", "Invite people")}
					</Button>
				)
			}
		>
			<ul className="flex flex-col gap-1">
				{team.people.map((person) => {
					const self = person.user_id === me;
					return (
						<li key={person.user_id} className="flex items-center gap-2">
							<PersonFace userId={person.user_id} />
							<span className="flex min-w-0 flex-1 flex-col">
								<PersonName
									userId={person.user_id}
									className="truncate text-sm"
								/>
								{person.status === "pending" && (
									<span className="text-[11px] text-warning">
										{t("teams.org.invited", "Invited")}
									</span>
								)}
								{admin && !team.parent_id && person.status === "active" && (
									<ClassificationSelect team={team} userId={person.user_id} />
								)}
							</span>
							{admin && person.status === "active" ? (
								<Select
									value={person.role}
									onValueChange={(role) =>
										actions.setPersonRole.mutate(
											{
												teamId: team.id,
												userId: person.user_id,
												role: role as TeamRole,
											},
											fail,
										)
									}
								>
									<SelectTrigger
										size="sm"
										className="h-7 w-28 text-xs"
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
							) : (
								<span className="text-xs text-muted-foreground">
									{person.role === "admin"
										? t("teams.org.roleAdmin", "Admin")
										: t("teams.org.roleMember", "Member")}
								</span>
							)}
							{(admin || self) && (
								<Button
									variant="ghost"
									size="icon"
									className="size-7"
									aria-label={
										self
											? t("teams.org.leave", "Leave team")
											: t("teams.org.removePerson", "Remove from team")
									}
									onClick={() =>
										actions.removePerson.mutate(
											{ teamId: team.id, userId: person.user_id },
											fail,
										)
									}
								>
									{self ? (
										<LogOutIcon aria-hidden />
									) : (
										<Trash2Icon aria-hidden />
									)}
								</Button>
							)}
						</li>
					);
				})}
			</ul>
		</InspectorSection>
	);
}

function TeamBots({ team, admin }: Readonly<{ team: ITeam; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const { me, botOf, nameOf, actions, setDialog, select } = useOrgChart();
	const fail = { onError: (error: Error) => toast.error(error.message) };
	return (
		<InspectorSection
			title={t("teams.org.bots", "Bots")}
			action={
				isTeamMember(team, me) && (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 text-xs"
						onClick={() => setDialog({ kind: "addBot", teamId: team.id })}
					>
						<PlusIcon aria-hidden />
						{t("teams.org.addBot", "Add bot")}
					</Button>
				)
			}
		>
			<ul className="flex flex-col gap-1">
				{team.bots.map((entry) => {
					const bot = botOf(entry.bot_id);
					if (!bot) return null;
					const pending = entry.status === "pending";
					return (
						<li key={entry.bot_id} className="flex items-center gap-2">
							<button
								type="button"
								onClick={() => select({ kind: "bot", id: bot.id })}
								className="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
							>
								<OrgBotAvatar bot={bot} size={24} />
								<span className="flex min-w-0 flex-1 flex-col">
									<span className="truncate text-sm">{nameOf(bot)}</span>
									{pending && (
										<span className="text-[11px] text-warning">
											{t("teams.org.waitingFor", "Waiting for")}{" "}
											<PersonName userId={entry.owner_id} />
										</span>
									)}
								</span>
							</button>
							{canTakeOut(team, bot, me) && (
								<Button
									variant="ghost"
									size="sm"
									className="h-7 text-xs"
									onClick={() =>
										actions.removeBot.mutate(
											{ teamId: team.id, botId: bot.id },
											fail,
										)
									}
								>
									{pending && admin
										? t("teams.org.withdraw", "Withdraw")
										: t("teams.org.remove", "Remove")}
								</Button>
							)}
						</li>
					);
				})}
			</ul>
		</InspectorSection>
	);
}

function DirectAsks({
	team,
	admin,
}: Readonly<{ team: ITeam; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const { me, botOf, nameOf, actions } = useOrgChart();
	const update = useTeamUpdate(team);
	const waiting = isWaitingForConsent(team);
	return (
		<InspectorSection
			title={t("teams.org.directAsks", "Teammates ask each other directly")}
			hint={t(
				"teams.org.directAsksHint",
				"Without it, a bot's owner approves the first question to a teammate. Every owner agrees for their own bots.",
			)}
			action={
				<Switch
					checked={team.direct_asks}
					disabled={!admin}
					aria-label={t(
						"teams.org.directAsks",
						"Teammates ask each other directly",
					)}
					onCheckedChange={(checked) => update({ direct_asks: checked })}
				/>
			}
		>
			{team.direct_asks && (
				<ul className="flex flex-col gap-1">
					{activeBots(team).map((entry) => {
						const bot = botOf(entry.bot_id);
						if (!bot) return null;
						const mine = entry.owner_id === me;
						return (
							<li
								key={entry.bot_id}
								className="flex items-center gap-2 text-sm"
							>
								<OrgBotAvatar bot={bot} size={20} />
								<span className="min-w-0 flex-1 truncate">{nameOf(bot)}</span>
								<span
									className={
										entry.direct_agreed
											? "text-xs text-good"
											: "text-xs text-warning"
									}
								>
									{entry.direct_agreed
										? t("teams.org.agreed", "Agreed")
										: t("teams.org.waiting", "Waiting")}
								</span>
								{mine && (
									<Button
										variant="outline"
										size="sm"
										className="h-6 text-xs"
										onClick={() =>
											actions.setDirectAgreement.mutate(
												{
													teamId: team.id,
													botId: entry.bot_id,
													agreed: !entry.direct_agreed,
												},
												{ onError: (error) => toast.error(error.message) },
											)
										}
									>
										{entry.direct_agreed
											? t("teams.org.stopAgreeing", "Stop")
											: t("teams.org.agree", "Agree")}
									</Button>
								)}
							</li>
						);
					})}
					{waiting && (
						<li className="text-xs text-muted-foreground">
							{t(
								"teams.org.waitingHint",
								"Bots whose owner has not agreed still ask with a card.",
							)}
						</li>
					)}
				</ul>
			)}
		</InspectorSection>
	);
}

function OwnClassification({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { org, me } = useOrgChart();
	const orgId = rootTeamId(org, team.id);
	const root = orgId ? teamById(org, orgId) : undefined;
	if (!me || !orgId || !root) return null;
	return (
		<p className="flex items-center gap-1 text-[11px] text-muted-foreground">
			<LockIcon className="size-3 shrink-0" aria-hidden />
			{t(
				"teams.reach.yourClassification",
				"Your classification in {{org}}: {{level}}",
				{ org: root.name, level: classificationOf(org, orgId, me) },
			)}
		</p>
	);
}

function DeleteTeam({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { actions, select } = useOrgChart();
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button
				variant="ghost"
				size="sm"
				className="self-start text-destructive hover:text-destructive"
				onClick={() => setOpen(true)}
			>
				<Trash2Icon aria-hidden />
				{t("teams.org.deleteTeam", "Delete team")}
			</Button>
			<AlertDialog open={open} onOpenChange={setOpen}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("teams.org.deleteTitle", "Delete {{name}}?", {
								name: team.name,
							})}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"teams.org.deleteBody",
								"Its bots and people leave the team. Teams inside it move up one level. The team room stays as a plain group.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("teams.org.cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={() =>
								actions.deleteTeam.mutate(team.id, {
									onSuccess: () => select(null),
									onError: (error) => toast.error(error.message),
								})
							}
						>
							{t("teams.org.deleteConfirm", "Delete team")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

export function TeamInspector({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { org, me, setDialog, onOpenConversation } = useOrgChart();
	const admin = isTeamAdmin(team, me);
	const parent = team.parent_id
		? org.teams.find((entry) => entry.id === team.parent_id)
		: undefined;
	const counts = teamCounts(team);
	return (
		<div className="flex flex-col gap-5">
			<header className="flex items-center gap-3">
				<TeamIcon team={team} />
				<div className="flex min-w-0 flex-1 flex-col">
					<h2 className="truncate text-base font-semibold">{team.name}</h2>
					<p className="truncate text-xs text-muted-foreground">
						{parent
							? t(
									"teams.org.teamIn",
									"Team in {{parent}} · {{bots}} bots · {{people}} people",
									{
										parent: parent.name,
										bots: counts.bots,
										people: counts.people,
									},
								)
							: t(
									"teams.org.topLevelSummary",
									"Top-level team · {{bots}} bots · {{people}} people",
									{ bots: counts.bots, people: counts.people },
								)}
					</p>
				</div>
			</header>
			<OwnClassification team={team} />
			{team.conversation_id && (
				<Button
					size="sm"
					variant="outline"
					className="self-start"
					onClick={() => onOpenConversation(team.conversation_id as string)}
				>
					<UsersIcon aria-hidden />
					{t("teams.org.openRoom", "Open team room")}
				</Button>
			)}
			<NameAndPurpose team={team} admin={admin} />
			<Placement team={team} admin={admin} />
			<People team={team} admin={admin} />
			<TeamBots team={team} admin={admin} />
			<TeamAppsSection team={team} />
			<TeamIoSection team={team} />
			<DirectAsks team={team} admin={admin} />
			{admin && (
				<div className="flex flex-col gap-2">
					<Button
						variant="outline"
						size="sm"
						className="self-start"
						onClick={() => setDialog({ kind: "newTeam", parentId: team.id })}
					>
						<PlusIcon aria-hidden />
						{t("teams.org.newTeamInside", "New team inside")}
					</Button>
					<DeleteTeam team={team} />
				</div>
			)}
		</div>
	);
}
