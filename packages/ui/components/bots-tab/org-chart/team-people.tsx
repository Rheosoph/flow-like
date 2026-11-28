"use client";

import { useTranslation } from "@flow-like/locales";
import { ClockIcon, UserPlusIcon } from "lucide-react";
import {
	type PointerEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { isTeamAdmin, isTeamMember } from "../../../lib/bot-teams/org-model";
import { cn } from "../../../lib/utils";
import type {
	ITeam,
	ITeamPerson,
} from "../../../state/backend-state/bot-team-state";
import { Button } from "../../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";
import { useOrgChart } from "./org-chart-context";
import { PersonFace, PersonName } from "./org-chart-parts";

const SHOWN_FACES = 3;
const OPEN_DELAY_MS = 150;
const CLOSE_DELAY_MS = 120;

/** The team's active people, admins first, in the order they joined. */
export function activePeople(team: ITeam): ITeamPerson[] {
	const active = team.people.filter((person) => person.status === "active");
	return [
		...active.filter((person) => person.role === "admin"),
		...active.filter((person) => person.role !== "admin"),
	];
}

/** Opens on mouse hover with a short delay and stays open while the pointer is on the card; taps toggle. */
function useHoverOpen() {
	const [open, setOpen] = useState(false);
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const clear = useCallback(() => {
		if (timer.current) clearTimeout(timer.current);
		timer.current = null;
	}, []);
	const later = useCallback(
		(next: boolean, delay: number) => (event: PointerEvent) => {
			if (event.pointerType !== "mouse") return;
			clear();
			timer.current = setTimeout(() => setOpen(next), delay);
		},
		[clear],
	);
	useEffect(() => clear, [clear]);
	return {
		open,
		setOpen: (next: boolean) => {
			clear();
			setOpen(next);
		},
		enter: later(true, OPEN_DELAY_MS),
		leave: later(false, CLOSE_DELAY_MS),
		keep: (event: PointerEvent) => {
			if (event.pointerType === "mouse") clear();
		},
	};
}

function PeopleCard({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { me, botOf, nameOf, setDialog } = useOrgChart();
	const people = activePeople(team);
	const admin = isTeamAdmin(team, me);
	const invited = team.people.filter(
		(person) => person.status === "pending",
	).length;
	const lead = team.lead_bot_id ? botOf(team.lead_bot_id) : undefined;
	return (
		<div className="flex flex-col">
			<div className="flex items-baseline justify-between gap-2 px-3 pt-3 pb-1.5">
				<span className="truncate text-sm font-semibold">
					{t("teams.org.peopleIn", "People in {{team}}", { team: team.name })}
				</span>
				<span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
					{t("teams.org.peopleCount", "{{count}} people", {
						count: people.length,
					})}
				</span>
			</div>
			<ul className="flex max-h-72 flex-col overflow-y-auto px-1.5 pb-1.5">
				{people.map((person) => (
					<li
						key={person.user_id}
						className="flex items-center gap-2 rounded-md px-1.5 py-1"
					>
						<PersonFace userId={person.user_id} size={26} />
						<span className="min-w-0 flex-1 truncate text-xs font-medium">
							<PersonName userId={person.user_id} />
							{person.user_id === me && (
								<span className="font-normal text-muted-foreground">
									{" "}
									{t("teams.org.you", "(you)")}
								</span>
							)}
						</span>
						{person.role === "admin" && (
							<span className="shrink-0 rounded-full border px-1.5 text-[10px] leading-4 text-muted-foreground">
								{t("teams.org.roleAdmin", "Admin")}
							</span>
						)}
					</li>
				))}
			</ul>
			{admin && invited > 0 && (
				<p className="flex items-center gap-1.5 border-t px-3 py-2 text-[11px] text-muted-foreground">
					<ClockIcon className="size-3" aria-hidden />
					{t("teams.org.invitedWaiting", "{{count}} invited, not joined yet", {
						count: invited,
					})}
				</p>
			)}
			{!isTeamMember(team, me) && (
				<p className="border-t px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
					{lead
						? t(
								"teams.org.notInTeamLead",
								"You're not in {{team}}. You can always talk to its lead, {{lead}}.",
								{ team: team.name, lead: nameOf(lead) },
							)
						: t("teams.org.notInTeam", "You're not in {{team}}.", {
								team: team.name,
							})}
				</p>
			)}
			{admin && (
				<div className="border-t p-2">
					<Button
						variant="outline"
						size="sm"
						className="h-7 w-full text-xs"
						onClick={() => setDialog({ kind: "addPeople", teamId: team.id })}
					>
						<UserPlusIcon aria-hidden />
						{t("teams.org.addPeople", "Add people")}
					</Button>
				</div>
			)}
		</div>
	);
}

/** A few stacked faces in the team header, apart from the bots; the card lists everyone. */
export function TeamPeopleStack({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const hover = useHoverOpen();
	const people = activePeople(team);
	if (people.length === 0) return null;
	const shown = people.slice(0, SHOWN_FACES);
	const more = people.length - shown.length;
	return (
		<Popover open={hover.open} onOpenChange={hover.setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-label={t(
						"teams.org.peopleButton",
						"People in {{team}}: {{count}}",
						{
							team: team.name,
							count: people.length,
						},
					)}
					onPointerEnter={hover.enter}
					onPointerLeave={hover.leave}
					className={cn(
						"flex h-6.5 shrink-0 items-center rounded-full py-0 pr-1 pl-2.5 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
						hover.open && "bg-muted",
					)}
				>
					{shown.map((person) => (
						<span
							key={person.user_id}
							className="-ml-1.5 flex rounded-full ring-2 ring-card"
						>
							<PersonFace userId={person.user_id} size={20} />
						</span>
					))}
					{more > 0 && (
						<span className="-ml-1.5 grid h-5 min-w-5 place-items-center rounded-full bg-muted px-1 text-[9px] font-semibold text-muted-foreground tabular-nums ring-2 ring-card">
							+{more}
						</span>
					)}
				</button>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				className="w-66 p-0"
				onPointerEnter={hover.keep}
				onPointerLeave={hover.leave}
				onOpenAutoFocus={(event) => event.preventDefault()}
			>
				<PeopleCard team={team} />
			</PopoverContent>
		</Popover>
	);
}
