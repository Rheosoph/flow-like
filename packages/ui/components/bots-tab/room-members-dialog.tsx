"use client";

import { useTranslation } from "@flow-like/locales";
import {
	LockIcon,
	LogOutIcon,
	SlidersHorizontalIcon,
	UserMinusIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useOrg } from "../../hooks/use-bot-teams";
import { useBots } from "../../hooks/use-bots";
import {
	useCancelInvitation,
	useInviteToConversation,
	useRemoveConversationMember,
	useSetMemberLimits,
} from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import { cn } from "../../lib/utils";
import type {
	IBotRoomLimits,
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
import { silentInConversation } from "./gated-bot-model";
import { LeaveRoomDialog } from "./leave-room-dialog";
import { SealingState } from "./new-room-dialog";
import { sealBlockText, useRoomSealing } from "./new-room-encryption";
import { PrincipalBlockButton } from "./principal-block-action";
import {
	type PickedPrincipal,
	type PickerSource,
	PrincipalPicker,
} from "./principal-picker";
import { RoomBotSetup, ownRoomBots, selectedBotLimits } from "./room-bot-setup";
import { RoomInviteLinks } from "./room-invite-links";
import { RoomLimitsDialog } from "./room-limits-dialog";
import type { CrewDirectory } from "./use-crew-directory";

function PersonRow({
	id,
	directory,
	note,
	onShow,
	children,
}: Readonly<{
	id: string;
	directory: CrewDirectory;
	note?: string;
	/** Opens who they are. */
	onShow?: (principalId: string) => void;
	children?: React.ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const name = directory(id).name;
	const who = (
		<>
			<IdentityAvatar subject={id} size={28} className="shrink-0" />
			<span className="flex min-w-0 flex-1 flex-col text-left">
				<span className="truncate text-sm font-medium">{name}</span>
				{note && (
					<span className="truncate text-xs text-muted-foreground">{note}</span>
				)}
			</span>
		</>
	);
	return (
		<li className="flex items-center gap-3 py-1.5">
			{onShow ? (
				<button
					type="button"
					onClick={() => onShow(id)}
					aria-label={t("crew.memberInfo.label", "About {{name}}", { name })}
					className="-mx-1.5 flex min-w-0 flex-1 touch-manipulation items-center gap-3 rounded-lg px-1.5 py-1 outline-none transition-[background-color,transform] duration-100 hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.98] active:bg-muted"
				>
					{who}
				</button>
			) : (
				<IdentityHoverCard
					subject={id}
					className="flex min-w-0 flex-1 items-center gap-3"
				>
					{who}
				</IdentityHoverCard>
			)}
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
	onShowMember,
}: Readonly<{
	conversation: IConversation;
	directory: CrewDirectory;
	ownBotIds: ReadonlySet<string>;
	me?: string | null;
	onEditLimits: (member: IConversationMember) => void;
	onRemove: (member: IConversationMember) => void;
	onShowMember?: (principalId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const isMember = conversation.members.some(
		(member) =>
			member.kind === "user" && isSelfPrincipal(member.principal_id, me),
	);
	const creator = isMember && isSelfPrincipal(conversation.created_by, me);
	const { org } = useOrg();
	const memberNote = (member: IConversationMember) => {
		if (member.kind !== "bot") return undefined;
		return silentInConversation(org, conversation.id, member.principal_id, me)
			? t("teams.chats.botSilent", "Bot · Doesn't answer here")
			: t("roomMembers.bot", "Bot");
	};
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
					note={memberNote(member)}
					onShow={
						onShowMember && !isSelfPrincipal(member.principal_id, me)
							? onShowMember
							: undefined
					}
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
					const inviter = member.invited_by ?? "";
					const mayCancel =
						isSelfPrincipal(inviter, me) ||
						isSelfPrincipal(conversation.created_by, me) ||
						(!!inviter && isSelfPrincipal(directory(inviter).ownerId, me));
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
	className,
}: Readonly<{
	conversation: IConversation;
	source: PickerSource;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const invite = useInviteToConversation();
	const [picked, setPicked] = useState<PickedPrincipal[]>([]);
	const [settingUp, setSettingUp] = useState(false);
	const [botLimits, setBotLimits] = useState<Record<string, IBotRoomLimits>>(
		{},
	);
	const ownBots = ownRoomBots(picked);
	const selected = new Set(picked.map((principal) => principal.id));
	const encrypted = conversation.encryption === "e2e";
	const sealing = useRoomSealing(picked, source.bots, encrypted);
	const { blockOf } = sealing;
	const reason = useCallback(
		(principal: PickedPrincipal) => {
			const block = blockOf(principal);
			return block ? sealBlockText(block, t) : undefined;
		},
		[blockOf, t],
	);
	const blocked = encrypted
		? picked.filter((principal) => blockOf(principal))
		: [];
	const unsealable =
		encrypted && (blocked.length > 0 || sealing.checking || sealing.failed);
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
				request: {
					principal_ids: picked.map((principal) => principal.id),
					...(ownBots.length > 0
						? { bot_limits: selectedBotLimits(picked, botLimits) }
						: {}),
				},
			})
			.then(({ added, invited }) => {
				setPicked([]);
				setSettingUp(false);
				setBotLimits({});
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
		<section className={cn("flex min-h-0 flex-col gap-3", className)}>
			<h3 className="shrink-0 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
				{t("roomMembers.invite", "Bring someone in")}
			</h3>
			{encrypted && (
				<p className="flex gap-1.5 text-xs text-muted-foreground">
					<LockIcon className="mt-0.5 size-3 shrink-0" aria-hidden />
					{t(
						"roomMembers.encryptedInvite",
						"This room is end-to-end encrypted. Whoever you bring in needs an encryption key first, and reads only what is sent after they join, never the earlier messages.",
					)}
				</p>
			)}
			{settingUp ? (
				<div className="min-h-0 lg:flex-1 lg:overflow-y-auto lg:pr-1">
					<RoomBotSetup
						bots={ownBots}
						limits={botLimits}
						disabled={invite.isPending}
						onChange={(id, limits) =>
							setBotLimits((current) => ({ ...current, [id]: limits }))
						}
					/>
				</div>
			) : (
				<PrincipalPicker
					className="lg:flex-1"
					listClassName="max-h-[min(28rem,55dvh)] lg:max-h-none lg:flex-1"
					source={source}
					selected={selected}
					multiple
					serverRule={{
						conversationId: conversation.id,
						memberIds: conversation.members.map(
							(member) => member.principal_id,
						),
						invitedIds: (conversation.pending ?? []).map(
							(member) => member.principal_id,
						),
					}}
					unavailable={encrypted ? reason : undefined}
					onPick={toggle}
				/>
			)}
			{encrypted && (
				<SealingState
					blocked={blocked}
					reason={reason}
					checking={sealing.checking}
					failed={sealing.failed}
					urgent
				/>
			)}
			<div className="flex shrink-0 flex-wrap justify-end gap-2">
				{settingUp && (
					<Button
						type="button"
						variant="outline"
						disabled={invite.isPending}
						onClick={() => setSettingUp(false)}
					>
						{t("roomLimits.setup.back", "Back")}
					</Button>
				)}
				<Button
					type="button"
					disabled={picked.length === 0 || unsealable || invite.isPending}
					onClick={() =>
						ownBots.length > 0 && !settingUp ? setSettingUp(true) : void send()
					}
				>
					{ownBots.length > 0 && !settingUp
						? t("roomLimits.setup.continue", "Continue to bot setup")
						: t("roomMembers.send", "Add or invite {{count}}", {
								count: picked.length,
							})}
				</Button>
			</div>
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
	onShowMember,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	conversation: IConversation;
	directory: CrewDirectory;
	source: PickerSource;
	me?: string | null;
	onLeave?: () => void;
	/** Closes the dialog and opens who the member is. */
	onShowMember?: (principalId: string) => void;
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
				<DialogContent className="flex h-[min(90dvh,52rem)] flex-col overflow-hidden sm:max-w-2xl lg:max-w-5xl">
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
					<div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto pr-1 lg:grid lg:grid-cols-[minmax(0,1fr)_minmax(0,1.25fr)] lg:grid-rows-[minmax(0,1fr)] lg:overflow-hidden lg:pr-0">
						<div className="max-lg:contents lg:flex lg:min-h-0 lg:flex-col lg:gap-4 lg:overflow-y-auto lg:pr-2">
							<MembersList
								conversation={conversation}
								directory={directory}
								ownBotIds={ownBotIds}
								me={me}
								onEditLimits={setEditing}
								onRemove={setRemoving}
								onShowMember={
									onShowMember &&
									((principalId) => {
										onOpenChange(false);
										onShowMember(principalId);
									})
								}
							/>
							<PendingList
								conversation={conversation}
								directory={directory}
								me={me}
							/>
							{open && conversation.kind === "group" && (
								<div className="max-lg:order-last">
									<RoomInviteLinks conversation={conversation} me={me} />
								</div>
							)}
						</div>
						{open && (
							<InviteSection
								conversation={conversation}
								source={source}
								className="max-lg:shrink-0 lg:border-l lg:border-border lg:pl-6"
							/>
						)}
					</div>
					{conversation.kind === "group" && self && (
						<Button
							type="button"
							variant="outline"
							className="shrink-0 text-destructive hover:text-destructive"
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
