"use client";

import { i18n as i18next } from "@flow-like/locales";
import { PencilLineIcon } from "lucide-react";
import { memo } from "react";
import { type PeerUserInfo, colorFromSub } from "../../hooks/use-peer-users";
import type { RemoteSelectionParticipant } from "./flow-node";
import type { RemoteEditorParticipant } from "./flowscript/flowscript-presence";

/**
 * One teammate touching this node, from the canvas and/or the code editor,
 * merged per user. A selection of code is a selection of nodes: both draw the
 * same ring and the same chip, the code one just carries a pencil.
 */
export interface NodePresenceParticipant {
	key: string;
	clientId: number;
	sub?: string;
	self?: boolean;
	/** Just clicked on the canvas, or holds the text cursor in the editor. */
	active: boolean;
	/** Selected on the canvas or spanned by a text selection — draws the ring. */
	selected: boolean;
	/** Reaches this node from the code editor (cursor, selection or unapplied edit). */
	inCode: boolean;
}

const EMPTY_NODE_PRESENCE: {
	list: NodePresenceParticipant[];
	ring?: { color: string; active: boolean };
} = { list: [] };

export function mergePresenceParticipants(
	remoteSelections?: readonly RemoteSelectionParticipant[],
	remoteEditors?: readonly RemoteEditorParticipant[],
): {
	list: NodePresenceParticipant[];
	ring?: { color: string; active: boolean };
} {
	if (!remoteSelections?.length && !remoteEditors?.length)
		return EMPTY_NODE_PRESENCE;
	const byUser = new Map<string, NodePresenceParticipant>();
	for (const participant of remoteSelections ?? []) {
		const key = participant.sub ?? `client:${participant.clientId}`;
		byUser.set(key, {
			key,
			clientId: participant.clientId,
			sub: participant.sub,
			self: participant.self,
			active: Boolean(participant.isActive),
			selected: true,
			inCode: false,
		});
	}
	for (const editor of remoteEditors ?? []) {
		const key = editor.sub ?? `client:${editor.clientId}`;
		const existing = byUser.get(key);
		byUser.set(key, {
			key,
			clientId: editor.clientId,
			sub: editor.sub,
			self: existing?.self || editor.self,
			active: Boolean(existing?.active) || editor.active,
			selected: Boolean(existing?.selected) || editor.selected,
			inCode: true,
		});
	}
	const list = [...byUser.values()].sort(
		(a, b) =>
			Number(b.active) - Number(a.active) ||
			Number(b.selected) - Number(a.selected) ||
			(a.sub ?? "").localeCompare(b.sub ?? ""),
	);
	const ringOwner = list.find((participant) => participant.selected);
	return {
		list,
		ring: ringOwner
			? {
					color: colorFromSub(ringOwner.sub),
					active: list.some((participant) => participant.active),
				}
			: undefined,
	};
}

export const NodePresenceChips = memo(function NodePresenceChips({
	participants,
	peerUsers,
}: {
	participants: NodePresenceParticipant[];
	peerUsers?: Map<string, PeerUserInfo>;
}) {
	if (participants.length === 0) return null;
	const shown = participants.slice(0, 3);
	const extra = participants.length - shown.length;
	return (
		<div className="pointer-events-none absolute -top-5 left-0 z-10 flex items-center gap-0.5">
			<div className="flex items-center -space-x-1.5">
				{shown.map((participant) => {
					const color = colorFromSub(participant.sub);
					const userInfo = participant.sub
						? peerUsers?.get(participant.sub)
						: undefined;
					const name = participant.self
						? i18next.t("flow:you", "You")
						: (userInfo?.truncatedName ?? "User");
					const title = participant.inCode
						? i18next.t("flow:flowscriptBeingEditedBy", {
								defaultValue: "Being edited by {{name}}",
								name,
							})
						: name;
					return (
						<div
							key={participant.key}
							className={`flex items-center gap-1 rounded-full border-2 bg-background/95 px-1 py-0.5 text-[0.5625rem] leading-none shadow-md backdrop-blur-sm transition-all duration-200 ${participant.active ? "animate-pulse scale-110" : ""}`}
							style={{ borderColor: color }}
							title={title}
						>
							{userInfo?.avatarUrl ? (
								<img
									src={userInfo.avatarUrl}
									alt={name}
									className="h-3.5 w-3.5 rounded-full object-cover"
								/>
							) : (
								<span
									className="flex h-3.5 w-3.5 items-center justify-center rounded-full text-[8px] font-bold text-white"
									style={{
										background: `linear-gradient(135deg, ${color}, ${color}dd)`,
									}}
								>
									{name.charAt(0).toUpperCase()}
								</span>
							)}
							{shown.length <= 2 && (
								<span
									className="font-semibold max-w-14 truncate pr-0.5"
									style={{ color }}
								>
									{name}
								</span>
							)}
							{participant.inCode && (
								<PencilLineIcon className="h-2.5 w-2.5" style={{ color }} />
							)}
						</div>
					);
				})}
			</div>
			{extra > 0 && (
				<div className="rounded-full border border-border bg-background/95 px-1.5 py-0.5 text-[0.5625rem] font-medium leading-none shadow-md">{`+${extra}`}</div>
			)}
		</div>
	);
});
