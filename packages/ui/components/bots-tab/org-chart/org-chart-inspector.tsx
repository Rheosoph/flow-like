"use client";

import { useTranslation } from "@flow-like/locales";
import { MousePointerClickIcon } from "lucide-react";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetTitle,
} from "../../ui/sheet";
import { BotInspector } from "./bot-inspector";
import { useOrgChart } from "./org-chart-context";
import { TeamInspector } from "./team-inspector";

function InspectorBody() {
	const { t } = useTranslation("bots");
	const { selection, org, botOf } = useOrgChart();
	if (selection?.kind === "bot") {
		const bot = botOf(selection.id);
		if (bot) return <BotInspector key={bot.id} bot={bot} />;
	}
	if (selection?.kind === "team") {
		const team = org.teams.find((entry) => entry.id === selection.id);
		if (team) return <TeamInspector key={team.id} team={team} />;
	}
	return (
		<div className="flex flex-col items-center gap-2 py-16 text-center text-sm text-muted-foreground">
			<MousePointerClickIcon className="size-5" aria-hidden />
			<p className="max-w-56">
				{t(
					"teams.org.selectHint",
					"Select a bot or a team to see its reporting line, who it can ask and what it reads.",
				)}
			</p>
		</div>
	);
}

export function DockedInspector() {
	const { t } = useTranslation("bots");
	return (
		<aside
			aria-label={t("teams.org.inspector", "Details")}
			className="w-[380px] shrink-0 overflow-y-auto border-l bg-card p-4"
		>
			<InspectorBody />
		</aside>
	);
}

export function SheetInspector() {
	const { t } = useTranslation("bots");
	const { selection, select } = useOrgChart();
	return (
		<Sheet
			open={selection !== null}
			onOpenChange={(open) => !open && select(null)}
		>
			<SheetContent side="bottom" className="max-h-[85dvh] overflow-y-auto p-4">
				<SheetTitle className="sr-only">
					{t("teams.org.inspector", "Details")}
				</SheetTitle>
				<SheetDescription className="sr-only">
					{t("teams.org.inspector", "Details")}
				</SheetDescription>
				<InspectorBody />
			</SheetContent>
		</Sheet>
	);
}
