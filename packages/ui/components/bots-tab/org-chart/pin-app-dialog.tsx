"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	HourglassIcon,
	Loader2Icon,
	PinIcon,
	SearchIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { usePinnableApps } from "../../../hooks/use-bot-teams";
import { apiErrorMessage } from "../../../lib/api-error";
import { appTypeLabel } from "../../../lib/app-type";
import {
	isTeamAdmin,
	rootTeamId,
	teamById,
} from "../../../lib/bot-teams/org-model";
import type { IAppType } from "../../../lib/schema/app/app";
import { initials, seedGradient } from "../../../lib/seed-gradient";
import { cn } from "../../../lib/utils";
import type {
	IPinnableApp,
	ITeam,
} from "../../../state/backend-state/bot-team-state";
import { AppTypeMark } from "../../ui/app-type-mark";
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
import { AppFilterPicker } from "./app-card";
import { type AppFilter, matchesFilter } from "./org-chart-apps";
import { useOrgChart } from "./org-chart-context";
import { PersonName } from "./org-chart-parts";

function PinAction({
	team,
	app,
	admin,
}: Readonly<{ team: ITeam; app: IPinnableApp; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const { actions } = useOrgChart();
	if (app.pinned === "active") {
		return (
			<span className="inline-flex h-7 items-center gap-1 rounded-md bg-good-bg px-2 text-xs font-medium text-good">
				<CheckIcon className="size-3" aria-hidden />
				{t("teams.org.apps.pinned", "Pinned")}
			</span>
		);
	}
	if (app.pinned === "pending") {
		return (
			<span className="inline-flex h-7 items-center gap-1 rounded-md bg-warning-bg px-2 text-xs font-medium text-warning">
				<HourglassIcon className="size-3" aria-hidden />
				{t("teams.org.apps.waiting", "Waiting")}
			</span>
		);
	}
	if (!app.reader_admin) {
		return (
			<span className="max-w-40 text-right text-[11px] text-muted-foreground">
				{app.owner_id ? (
					<>
						{t(
							"teams.org.apps.onlyAdminsOf",
							"Only its admins can pin it, like",
						)}{" "}
						<PersonName userId={app.owner_id} />
					</>
				) : (
					t("teams.org.apps.onlyAdmins", "Only its admins can pin it")
				)}
			</span>
		);
	}
	return (
		<Button
			size="sm"
			variant="outline"
			className="h-7 text-xs"
			disabled={actions.pinApp.isPending}
			onClick={() =>
				actions.pinApp.mutate(
					{ teamId: team.id, appId: app.id },
					{
						onSuccess: () =>
							toast.success(
								admin
									? t(
											"teams.org.apps.pinnedToast",
											"{{app}} is pinned to {{team}}",
											{
												app: app.name,
												team: team.name,
											},
										)
									: t(
											"teams.org.apps.askedToast",
											"{{app}} shows in {{team}} once an admin approves",
											{ app: app.name, team: team.name },
										),
							),
						onError: (error) =>
							toast.error(
								apiErrorMessage(
									error,
									t("teams.org.apps.pinFailed", "Couldn't change the pin."),
								),
							),
					},
				)
			}
		>
			<PinIcon aria-hidden />
			{admin
				? t("teams.org.apps.pinShort", "Pin")
				: t("teams.org.apps.askToPin", "Ask to pin")}
		</Button>
	);
}

function PinnableRow({
	team,
	app,
	admin,
}: Readonly<{ team: ITeam; app: IPinnableApp; admin: boolean }>) {
	const { t } = useTranslation("bots");
	const { org } = useOrgChart();
	const elsewhere = app.pinned_elsewhere
		.map((id) => teamById(org, id)?.name)
		.filter(Boolean)
		.join(", ");
	return (
		<li
			className={cn(
				"flex items-center gap-3 rounded-lg p-2",
				!app.reader_admin && "opacity-75",
			)}
		>
			<AppTypeMark
				type={(app.app_type ?? null) as IAppType | null}
				size={32}
				src={app.icon}
				fallback={initials(app.name)}
				background={seedGradient(app.id)}
			/>
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate text-sm font-medium">{app.name}</span>
				<span className="truncate text-[11px] text-muted-foreground">
					{app.app_type
						? appTypeLabel(app.app_type as IAppType)
						: t("teams.org.apps.unclassified", "No type yet")}
					{elsewhere &&
						` · ${t("teams.org.apps.alsoIn", "also in {{teams}}", { teams: elsewhere })}`}
				</span>
			</span>
			<PinAction team={team} app={app} admin={admin} />
		</li>
	);
}

function Section({
	title,
	apps,
	team,
	admin,
}: Readonly<{
	title: string;
	apps: IPinnableApp[];
	team: ITeam;
	admin: boolean;
}>) {
	if (apps.length === 0) return null;
	return (
		<section className="flex flex-col">
			<h3 className="px-2 pt-2 pb-1 text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
				{title}
			</h3>
			<ul className="flex flex-col">
				{apps.map((app) => (
					<PinnableRow key={app.id} team={team} app={app} admin={admin} />
				))}
			</ul>
		</section>
	);
}

export function PinAppDialog({
	team,
	onClose,
}: Readonly<{ team: ITeam; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	const { org, me } = useOrgChart();
	const admin = isTeamAdmin(team, me);
	const orgName =
		teamById(org, rootTeamId(org, team.id) ?? team.id)?.name ?? "";
	const [query, setQuery] = useState("");
	const [filter, setFilter] = useState<AppFilter>("all");
	const pinnable = usePinnableApps(team.id, true);
	const shown = useMemo(() => {
		const needle = query.trim().toLowerCase();
		return (pinnable.data ?? []).filter(
			(app) =>
				matchesFilter(app, filter) &&
				(!needle ||
					app.name.toLowerCase().includes(needle) ||
					(app.description ?? "").toLowerCase().includes(needle)),
		);
	}, [pinnable.data, query, filter]);
	const administered = shown.filter((app) => app.reader_admin);
	const others = shown.filter((app) => !app.reader_admin);
	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{t("teams.org.apps.pinTitle", "Pin an app to {{team}}", {
							team: team.name,
						})}
					</DialogTitle>
					<DialogDescription>
						{admin
							? t(
									"teams.org.apps.pinHint",
									"Everyone in {{org}} sees the name, icon, description and type of apps you pin. Opening one still needs access.",
									{ org: orgName },
								)
							: t(
									"teams.org.apps.pinHintMember",
									"Everyone in {{org}} will see the name, icon, description and type. A team admin approves your pin before it shows.",
									{ org: orgName },
								)}
					</DialogDescription>
				</DialogHeader>
				<div className="flex flex-col gap-2">
					<div className="relative">
						<label htmlFor="pin-app-search" className="sr-only">
							{t("teams.org.apps.search", "Search your apps")}
						</label>
						<SearchIcon
							className="absolute top-2.5 left-2.5 size-4 text-muted-foreground"
							aria-hidden
						/>
						<Input
							id="pin-app-search"
							type="search"
							value={query}
							onChange={(event) => setQuery(event.target.value)}
							placeholder={t("teams.org.apps.search", "Search your apps")}
							className="pl-8"
						/>
					</div>
					<AppFilterPicker value={filter} onChange={setFilter} look="pills" />
				</div>
				<div className="-mx-2 max-h-[46vh] min-h-32 overflow-y-auto">
					{pinnable.isLoading ? (
						<div className="flex justify-center py-10 text-muted-foreground">
							<Loader2Icon className="size-4 animate-spin" aria-hidden />
						</div>
					) : pinnable.isError ? (
						<p className="px-2 py-8 text-center text-sm text-muted-foreground">
							{apiErrorMessage(
								pinnable.error,
								t("teams.org.apps.listFailed", "Couldn't load your apps."),
							)}
						</p>
					) : shown.length === 0 ? (
						<p className="px-2 py-8 text-center text-sm text-muted-foreground">
							{(pinnable.data ?? []).length === 0
								? t(
										"teams.org.apps.noneYet",
										"You're not in any app yet. Apps you own or administer can be pinned here.",
									)
								: t("teams.org.apps.noMatch", "No app matches.")}
						</p>
					) : (
						<>
							<Section
								title={t("teams.org.apps.yourAdmin", "Apps you administer")}
								apps={administered}
								team={team}
								admin={admin}
							/>
							<Section
								title={t(
									"teams.org.apps.yourMember",
									"Apps you're only a member of",
								)}
								apps={others}
								team={team}
								admin={admin}
							/>
						</>
					)}
				</div>
				<DialogFooter>
					<Button onClick={onClose}>{t("teams.org.done", "Done")}</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
