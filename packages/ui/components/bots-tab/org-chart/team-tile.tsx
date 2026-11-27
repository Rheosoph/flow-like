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
import { useState } from "react";
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

/**
 * A team's square, the same wherever the team shows: its room's picture, or a colored square
 * with an icon while the room has none or the picture fails to load.
 */
export function TeamTile({
	teamId,
	name,
	imageUrl,
	size = 28,
}: Readonly<{
	teamId: string;
	name?: string;
	imageUrl?: string | null;
	size?: number;
}>) {
	const [failedUrl, setFailedUrl] = useState<string | null>(null);
	const { tone, icon } = teamTile(teamId, name);
	const Icon = ICONS[icon];
	if (imageUrl && imageUrl !== failedUrl)
		return (
			<img
				src={imageUrl}
				alt=""
				aria-hidden
				draggable={false}
				onError={() => setFailedUrl(imageUrl)}
				className="shrink-0 rounded-lg object-cover"
				style={{ width: size, height: size }}
			/>
		);
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
