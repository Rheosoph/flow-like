"use client";

import {
	BriefcaseIcon,
	FlaskConicalIcon,
	HeadsetIcon,
	LandmarkIcon,
	type LucideIcon,
	MegaphoneIcon,
	ShieldIcon,
	UsersIcon,
	WrenchIcon,
} from "lucide-react";
import { cn } from "../../../lib/utils";
import { type TeamIconKey, type TeamTone, teamTile } from "./org-chart-model";

const TONE_CLASS: Record<TeamTone, string> = {
	info: "bg-info-bg text-info",
	good: "bg-good-bg text-good",
	warning: "bg-warning-bg text-warning",
	locked: "bg-locked-bg text-locked",
	paused: "bg-paused-bg text-paused",
};

const ICONS: Record<TeamIconKey, LucideIcon> = {
	briefcase: BriefcaseIcon,
	shield: ShieldIcon,
	headset: HeadsetIcon,
	users: UsersIcon,
	landmark: LandmarkIcon,
	flask: FlaskConicalIcon,
	megaphone: MegaphoneIcon,
	wrench: WrenchIcon,
};

/** A team's colored square with its icon, the same one wherever the team shows. */
export function TeamTile({
	teamId,
	name,
	size = 28,
}: Readonly<{ teamId: string; name?: string; size?: number }>) {
	const { tone, icon } = teamTile(teamId, name);
	const Icon = ICONS[icon];
	return (
		<span
			aria-hidden
			className={cn(
				"flex shrink-0 items-center justify-center rounded-lg",
				TONE_CLASS[tone],
			)}
			style={{ width: size, height: size }}
		>
			<Icon style={{ width: size * 0.55, height: size * 0.55 }} />
		</span>
	);
}
