"use client";

import { useTranslation } from "@flow-like/locales";
import {
	LogOutIcon,
	SlidersHorizontalIcon,
	UserMinusIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { useBots } from "../../hooks/use-bots";
import {
	useCancelInvitation,
	useInviteToConversation,
	useRemoveConversationMember,
	useSetMemberLimits,
} from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import type {
	IConversation,
	IConversationMember,
} from "../../state/backend-state/inbox-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { IdentityHoverCard } from "../identity/identity-hover-card";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { BotUsageButton } from "./bot-usage";
import { isSelfPrincipal } from "./crew-model";
import { LeaveRoomDialog } from "./leave-room-dialog";
import { PrincipalBlockButton } from "./principal-block-action";
import {
	type PickedPrincipal,
	type PickerSource,
	PrincipalPicker,
} from "./principal-picker";
import { RoomInviteLinks } from "./room-invite-links";
import { RoomLimitsDialog } from "./room-limits-dialog";
import type { CrewDirectory } from "./use-crew-directory";

function PersonRow({
	id,
	directory,
	note,
	children,
}: Readonly<{
	id: string;
	directory: CrewDirectory;
	note?: string;
	children?: React.ReactNode;
}>) {
	return (
		<li className="flex items-center gap-3 py-1.5">
			<IdentityHoverCard subject={id} className="shrink-0">
				<IdentityAvatar subject={id} size={28} />
			</IdentityHoverCard>
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate text-sm font-medium">
					{directory(id).name}
				</span>
				{note && (
					<span className="truncate text-xs text-muted-foreground">{note}</span>
				)}
			</span>
			{children}
		</li>
	);
}

function MembersList({
	conversation,
	directory,
	ownBotIds,
	me,
	onEditLimits,
	onRemove,
}: Readonly<{
	conversation: IConversation;
	directory: CrewDirectory;
	ownBotIds: ReadonlySet<string>;
	me?: string | null;
	onEditLimits: (member: IConversationMember) => void;
	onRemove: (member: IConversationMember) => void;
}>) {
	const { t } = useTranslation("bots");
	const isMember = conversation.members.some(
		(member) =>
			member.kind === "user" && isSelfPrincipal(member.principal_id, me),
	);
	const creator = isMember && isSelfPrincipal(conversation.created_by, me);
	return (
		<ul
			aria-label={t("roomMembers.members", "Members")}
			className="flex flex-col"
		>
			{conversation.members.map((member) => (
				<PersonRow
					key={member.principal_id}
					id={member.principal_id}
					directory={directory}
					note={member.kind === "bot" ? t("roomMembers.bot", "Bot") : undefined}
				>
					{member.kind === "bot" &&
						conversation.kind === "group" &&
						!conversation.local && (
							<BotUsageButton
								botId={member.principal_id}
								groupId={conversation.id}
								name={directory(member.principal_id).name}
								canEdit={ownBotIds.has(member.principal_id)}
							/>
						)}
					{member.kind === "bot" && ownBotIds.has(member.principal_id) && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							onClick={() => onEditLimits(member)}
						>
							<SlidersHorizontalIcon aria-hidden />
							{t("roomMembers.limits", "Room limits")}
						</Button>
					)}
					{me &&
						!isSelfPrincipal(member.principal_id, me) &&
						!(
							member.kind === "bot" &&
							(ownBotIds.has(member.principal_id) ||
								isSelfPrincipal(directory(member.principal_id).ownerId, me))
						) && (
							<PrincipalBlockButton
								principal={{
									id: member.principal_id,
									name: directory(member.principal_id).name,
									kind: member.kind,
									ownerId: directory(member.principal_id).ownerId,
								}}
							/>
						)}
					{conversation.kind === "group" &&
						isMember &&
						!isSelfPrincipal(member.principal_id, me) &&
						(creator ||
							(member.kind === "bot" &&
								ownBotIds.has(member.principal_id))) && (
							<Button
								type="button"
								variant="ghost"
								size="icon"
								aria-label={t(
									"roomMembers.removeNamed",
									"Remove {{name}} from room",
									{
										name: directory(member.principal_id).name,
									},
								)}
								onClick={() => onRemove(member)}
							>
								<UserMinusIcon aria-hidden />
							</Button>
						)}
				</PersonRow>
			))}
		</ul>
	);
}

function RemoveMemberDialog({
	conversationId,
	member,
	name,
	onClose,
}: Readonly<{
	conversationId: string;
	member: IConversationMember;
	name: string;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const remove = useRemoveConversationMember();
	const [error, setError] = useState<string | null>(null);
	const confirm = async () => {
		setError(null);
		try {
			await remove.mutateAsync({
				conversationId,
				principalId: member.principal_id,
			});
			onClose();
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t(
						"roomMembers.removeFailed",
						"Couldn't remove this member. Try again.",
					),
				),
			);
		}
	};
	return (
		<AlertDialog
			open
			onOpenChange={(open) => {
				if (!open && !remove.isPending) onClose();
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("roomMembers.removeTitle", "Remove {{name}} from this room?", {
							name,
						})}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{member.kind === "user"
							? t(
									"roomMembers.removePersonDescription",
									"This person and their bots in the room will leave and stop receiving its messages.",
								)
							: t(
									"roomMembers.removeBotDescription",
									"This bot will leave the room and stop receiving its messages.",
								)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<AlertDialogFooter>
					<AlertDialogCancel disabled={remove.isPending}>
						{t("crew.cancel", "Cancel")}
					</AlertDialogCancel>
					<Button
						type="button"
						variant="destructive"
						disabled={remove.isPending}
						onClick={() => void confirm()}
					>
						{t("roomMembers.remove", "Remove from room")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function PendingList({
	conversation,
	directory,
	me,
}: Readonly<{
	conversation: IConversation;
	directory: CrewDirectory;
	me?: string | null;
}>) {
	const { t } = useTranslation("bots");
	const cancel = useCancelInvitation();
	const pending = conversation.pending ?? [];
	if (pending.length === 0) return null;
	return (
		<section className="flex flex-col gap-1">
			<h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
				{t("roomMembers.pending", "Invited")}
			</h3>
			<ul className="flex flex-col">
				{pending.map((member) => {
					const mayCancel =
						isSelfPrincipal(member.invited_by ?? "", me) ||
						isSelfPrincipal(conversation.created_by, me);
					return (
						<PersonRow
							key={member.principal_id}
							id={member.principal_id}
							directory={directory}
							note={
								member.status === "declined"
									? t("roomMembers.declined", "Declined")
									: member.kind === "bot"
										? t(
												"roomMembers.waitingOwner",
												"Waiting for the bot's owner",
											)
										: t("roomMembers.waiting", "Waiting for an answer")
							}
						>
							{mayCancel && (
								<Button
									type="button"
									variant="ghost"
									size="icon"
									disabled={cancel.isPending}
									aria-label={t(
										"roomMembers.cancel",
										"Withdraw invitation of {{name}}",
										{
											name: directory(member.principal_id).name,
										},
									)}
									onClick={() =>
										cancel
											.mutateAsync({
												conversationId: conversation.id,
												principalId: member.principal_id,
											})
											.catch((error: unknown) =>
												toast.error(
													apiErrorMessage(
														error,
														t(
															"roomMembers.cancelFailed",
															"Couldn't withdraw the invitation.",
														),
													),
												),
											)
									}
								>
									<XIcon aria-hidden />
								</Button>
							)}
						</PersonRow>
					);
				})}
			</ul>
		</section>
	);
}

function InviteSection({
	conversation,
	source,
}: Readonly<{ conversation: IConversation; source: PickerSource }>) {
	const { t } = useTranslation("bots");
	const invite = useInviteToConversation();
	const [picked, setPicked] = useState<PickedPrincipal[]>([]);
	const selected = new Set(picked.map((principal) => principal.id));
	const toggle = (principal: PickedPrincipal) =>
		setPicked((list) =>
			list.some((item) => item.id === principal.id)
				? list.filter((item) => item.id !== principal.id)
				: [...list, principal],
		);
	const send = () =>
		invite
			.mutateAsync({
				conversationId: conversation.id,
				request: { principal_ids: picked.map((principal) => principal.id) },
			})
			.then(({ added, invited }) => {
				setPicked([]);
				toast.success(
					t("roomMembers.sent", "{{added}} added, {{invited}} invited", {
						added: added.length,
						invited: invited.length,
					}),
				);
			})
			.catch((error: unknown) =>
				toast.error(
					apiErrorMessage(
						error,
						t("roomMembers.inviteFailed", "Couldn't send the invitations."),
					),
				),
			);
	return (
		<section className="flex min-h-0 flex-col gap-3">
			<h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
				{t("roomMembers.invite", "Bring someone in")}
			</h3>
			<PrincipalPicker
				source={source}
				selected={selected}
				multiple
				serverRule={{
					conversationId: conversation.id,
					memberIds: conversation.members.map((member) => member.principal_id),
					invitedIds: (conversation.pending ?? []).map(
						(member) => member.principal_id,
					),
				}}
				onPick={toggle}
			/>
			<Button
				type="button"
				disabled={picked.length === 0 || invite.isPending}
				onClick={() => void send()}
			>
				{t("roomMembers.send", "Add or invite {{count}}", {
					count: picked.length,
				})}
			</Button>
		</section>
	);
}

/** Who is in a group, who was invited, the room limits of your bots, and the picker for more. */
export function RoomMembersDialog({
	open,
	onOpenChange,
	conversation,
	directory,
	source,
	me,
	onLeave,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	conversation: IConversation;
	directory: CrewDirectory;
	source: PickerSource;
	me?: string | null;
	onLeave?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const bots = useBots();
	const setLimits = useSetMemberLimits();
	const [editing, setEditing] = useState<IConversationMember | null>(null);
	const [removing, setRemoving] = useState<IConversationMember | null>(null);
	const [leaving, setLeaving] = useState(false);
	const self = conversation.members.find(
		(member) =>
			member.kind === "user" && isSelfPrincipal(member.principal_id, me),
	);
	const ownBotIds = useMemo(
		() => new Set((bots.data ?? []).map((bot) => bot.id)),
		[bots.data],
	);
	return (
		<>
			<Dialog open={open} onOpenChange={onOpenChange}>
				<DialogContent className="flex max-h-[85dvh] flex-col overflow-y-auto sm:max-w-md">
					<DialogHeader>
						<DialogTitle className="flex items-center gap-2">
							<UsersIcon className="size-4" aria-hidden />
							{t("roomMembers.title", "Room members")}
						</DialogTitle>
						<DialogDescription>
							{t(
								"roomMembers.description",
								"Invited people and other people's bots join once they accept.",
							)}
						</DialogDescription>
					</DialogHeader>
					<MembersList
						conversation={conversation}
						directory={directory}
						ownBotIds={ownBotIds}
						me={me}
						onEditLimits={setEditing}
						onRemove={setRemoving}
					/>
					<PendingList
						conversation={conversation}
						directory={directory}
						me={me}
					/>
					{open && (
						<InviteSection conversation={conversation} source={source} />
					)}
					{open && conversation.kind === "group" && (
						<RoomInviteLinks conversation={conversation} me={me} />
					)}
					{conversation.kind === "group" && self && (
						<Button
							type="button"
							variant="outline"
							className="text-destructive hover:text-destructive"
							onClick={() => setLeaving(true)}
						>
							<LogOutIcon aria-hidden />
							{t("crew.leaveRoom.action", "Leave room")}
						</Button>
					)}
				</DialogContent>
			</Dialog>
			{self && leaving && (
				<LeaveRoomDialog
					open
					onOpenChange={setLeaving}
					conversationId={conversation.id}
					principalId={self.principal_id}
					onLeave={() => {
						onOpenChange(false);
						onLeave?.();
					}}
				/>
			)}
			{removing && (
				<RemoveMemberDialog
					conversationId={conversation.id}
					member={removing}
					name={directory(removing.principal_id).name}
					onClose={() => setRemoving(null)}
				/>
			)}
			{editing && (
				<RoomLimitsDialog
					open
					onOpenChange={(next) => {
						if (!next) setEditing(null);
					}}
					botId={editing.principal_id}
					botName={directory(editing.principal_id).name}
					roomName={
						conversation.title ?? t("roomMembers.thisRoom", "this room")
					}
					initial={editing.limits}
					confirmLabel={t("roomMembers.saveLimits", "Save limits")}
					pending={setLimits.isPending}
					error={setLimits.error}
					onConfirm={(limits) =>
						setLimits
							.mutateAsync({
								conversationId: conversation.id,
								botId: editing.principal_id,
								limits,
							})
							.then(() => setEditing(null))
							.catch(() => undefined)
					}
				/>
			)}
		</>
	);
}
