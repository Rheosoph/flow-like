"use client";

import { useTranslation } from "@flow-like/locales";
import { ImagePlusIcon, PencilIcon, RotateCcwIcon } from "lucide-react";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { useSetTeamIcon } from "../../../hooks/use-bot-teams";
import { apiErrorMessage } from "../../../lib/api-error";
import { isTeamMember } from "../../../lib/bot-teams/org-model";
import type { ITeam } from "../../../state/backend-state/bot-team-state";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import {
	ROOM_IMAGE_ACCEPT,
	RoomImageError,
	prepareRoomImage,
} from "../room-image";
import { useRoomImageProblemText } from "../room-settings-dialog";
import { useOrgChart } from "./org-chart-context";
import { TeamTile } from "./team-tile";

function useTeamImage(team: ITeam, roomId: string) {
	const { t } = useTranslation("bots");
	const setIcon = useSetTeamIcon();
	const problemText = useRoomImageProblemText();
	const [preparing, setPreparing] = useState(false);

	const save = (image: string | null) =>
		setIcon.mutate(
			{ teamId: team.id, roomId, image },
			{
				onError: (error) =>
					toast.error(
						apiErrorMessage(
							error,
							t("teams.org.iconFailed", "Couldn't change the team's icon."),
						),
					),
			},
		);

	const pick = (file: File) => {
		setPreparing(true);
		prepareRoomImage(file)
			.then(save)
			.catch((cause: unknown) =>
				toast.error(
					problemText(
						cause instanceof RoomImageError ? cause.problem : "unreadable",
					),
				),
			)
			.finally(() => setPreparing(false));
	};

	return { pick, save, busy: preparing || setIcon.isPending };
}

function TeamIconMenu({
	team,
	roomId,
	size,
}: Readonly<{ team: ITeam; roomId: string; size: number }>) {
	const { t } = useTranslation("bots");
	const input = useRef<HTMLInputElement>(null);
	const { pick, save, busy } = useTeamImage(team, roomId);
	return (
		<>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<button
						type="button"
						disabled={busy}
						aria-label={t("teams.org.iconChange", "Change team icon")}
						className="group relative shrink-0 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
					>
						<TeamTile
							teamId={team.id}
							name={team.name}
							imageUrl={team.image_url}
							size={size}
						/>
						<span
							aria-hidden
							className="absolute -right-1 -bottom-1 flex size-5 items-center justify-center rounded-full border bg-background text-muted-foreground shadow-sm opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100 group-data-[state=open]:opacity-100"
						>
							<PencilIcon className="size-3" />
						</span>
					</button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="start" className="w-60">
					<DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
						{t("teams.org.iconHint", "The team room shows the same picture.")}
					</DropdownMenuLabel>
					<DropdownMenuSeparator />
					<DropdownMenuItem onSelect={() => input.current?.click()}>
						<ImagePlusIcon aria-hidden />
						{team.image_url
							? t("crew.roomSettings.change", "Change picture")
							: t("crew.roomSettings.upload", "Upload picture")}
					</DropdownMenuItem>
					{team.image_url && (
						<DropdownMenuItem onSelect={() => save(null)}>
							<RotateCcwIcon aria-hidden />
							{t("teams.org.iconDefault", "Use default icon")}
						</DropdownMenuItem>
					)}
				</DropdownMenuContent>
			</DropdownMenu>
			<input
				ref={input}
				type="file"
				accept={ROOM_IMAGE_ACCEPT}
				className="hidden"
				onChange={(event) => {
					const file = event.target.files?.[0];
					event.target.value = "";
					if (file) pick(file);
				}}
			/>
		</>
	);
}

/** The team's icon; people in the team change it, which changes the team room's picture too. */
export function TeamIcon({
	team,
	size = 44,
}: Readonly<{ team: ITeam; size?: number }>) {
	const { me } = useOrgChart();
	if (team.conversation_id && isTeamMember(team, me))
		return (
			<TeamIconMenu team={team} roomId={team.conversation_id} size={size} />
		);
	return (
		<TeamTile
			teamId={team.id}
			name={team.name}
			imageUrl={team.image_url}
			size={size}
		/>
	);
}
