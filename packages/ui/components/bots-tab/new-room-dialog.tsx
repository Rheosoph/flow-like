"use client";

import { useTranslation } from "@flow-like/locales";
import { LockIcon, XIcon } from "lucide-react";
import { useCallback, useId, useState } from "react";
import { useCreateConversation } from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import {
	type IBotRoomLimits,
	MAX_CONVERSATION_MEMBERS,
} from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";
import {
	canSealRooms,
	encryptedRoomError,
	sealBlockText,
	useRoomSealing,
} from "./new-room-encryption";
import {
	type PickedPrincipal,
	type PickerSource,
	PrincipalPicker,
} from "./principal-picker";
import { RoomBotSetup, ownRoomBots, selectedBotLimits } from "./room-bot-setup";

const MAX_OTHERS = MAX_CONVERSATION_MEMBERS - 1;

/** Who an end-to-end encrypted room can't take yet, and why; `urgent` once encryption is on. */
export function SealingState({
	blocked,
	reason,
	checking,
	failed,
	urgent,
}: Readonly<{
	blocked: readonly PickedPrincipal[];
	reason: (principal: PickedPrincipal) => string | undefined;
	checking: boolean;
	failed: boolean;
	urgent: boolean;
}>) {
	const { t } = useTranslation("bots");
	if (failed) {
		return (
			<p role="alert" className="text-xs text-destructive">
				{t(
					"crew.newRoomDialog.encryptionCheckFailed",
					"Couldn't check who has an encryption key. Try again.",
				)}
			</p>
		);
	}
	if (checking) {
		return urgent ? (
			<p className="text-xs text-muted-foreground">
				{t(
					"crew.newRoomDialog.encryptionChecking",
					"Checking everyone's encryption keys…",
				)}
			</p>
		) : null;
	}
	if (blocked.length === 0) return null;
	return (
		<div
			aria-live="polite"
			className={cn(
				"flex flex-col gap-0.5 text-xs",
				urgent ? "text-destructive" : "text-muted-foreground",
			)}
		>
			<p>
				{t(
					"crew.newRoomDialog.encryptionWaitsFor",
					"Possible once everyone can be sealed to:",
				)}
			</p>
			<ul className="flex flex-col gap-0.5">
				{blocked.map((principal) => (
					<li key={principal.id}>
						{principal.label}: {reason(principal)}
					</li>
				))}
			</ul>
		</div>
	);
}

function NewRoomForm({
	source,
	onCancel,
	onCreated,
}: Readonly<{
	source: PickerSource;
	onCancel: () => void;
	onCreated: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const titleId = useId();
	const sealedId = useId();
	const [title, setTitle] = useState("");
	const [picked, setPicked] = useState<PickedPrincipal[]>([]);
	const [settingUp, setSettingUp] = useState(false);
	const [botLimits, setBotLimits] = useState<Record<string, IBotRoomLimits>>(
		{},
	);
	const [sealed, setSealed] = useState(false);
	const inbox = useBackend().inboxState;
	const canSeal = canSealRooms(source.people, Boolean(inbox));
	const encrypted = canSeal && sealed;
	const ownBots = ownRoomBots(picked);
	const create = useCreateConversation();
	const sealing = useRoomSealing(picked, source.bots, canSeal);
	const { blockOf } = sealing;
	const reason = useCallback(
		(principal: PickedPrincipal) => {
			const block = blockOf(principal);
			return block ? sealBlockText(block, t) : undefined;
		},
		[blockOf, t],
	);
	const blocked = canSeal
		? picked.filter((principal) => blockOf(principal))
		: [];
	const sealable = blocked.length === 0 && !sealing.checking && !sealing.failed;
	const failed = t("crew.newRoomDialog.failed", "Couldn't create the room.");
	const selected = new Set(picked.map((principal) => principal.id));
	// A chat on this device holds you and its bots only; the account can't reach them.
	const invites = picked.filter(
		(principal) => principal.mode === "invite",
	).length;
	const mixesLocal =
		picked.some((principal) => principal.bot?.local) &&
		picked.some((principal) => !principal.bot?.local);

	const toggle = (principal: PickedPrincipal) =>
		setPicked((list) =>
			list.some((item) => item.id === principal.id)
				? list.filter((item) => item.id !== principal.id)
				: list.length >= MAX_OTHERS
					? list
					: [...list, principal],
		);

	const submit = async () => {
		if (picked.length === 0 || mixesLocal || (encrypted && !sealable)) return;
		if (ownBots.length > 0 && !settingUp) {
			setSettingUp(true);
			return;
		}
		const conversation = await create.mutateAsync({
			kind: "group",
			title: title.trim() || null,
			members: picked.map((principal) => principal.id),
			...(encrypted ? { encryption: "e2e" as const } : {}),
			...(ownBots.length > 0
				? { bot_limits: selectedBotLimits(picked, botLimits) }
				: {}),
		});
		onCreated(conversation.id);
	};

	return (
		<form
			className="flex min-h-0 flex-col gap-4"
			onSubmit={(event) => {
				event.preventDefault();
				submit().catch(() => undefined);
			}}
		>
			<div className="min-h-0 max-h-[60dvh] overflow-y-auto pr-1">
				{settingUp ? (
					<RoomBotSetup
						bots={ownBots}
						limits={botLimits}
						disabled={create.isPending}
						onChange={(id, limits) =>
							setBotLimits((current) => ({ ...current, [id]: limits }))
						}
					/>
				) : (
					<div className="flex min-h-0 flex-col gap-4">
						<div className="flex flex-col gap-1.5">
							<Label htmlFor={titleId}>
								{t("crew.newRoomDialog.name", "Name")}
							</Label>
							<Input
								id={titleId}
								value={title}
								maxLength={120}
								onChange={(event) => setTitle(event.target.value)}
								placeholder={t(
									"crew.newRoomDialog.namePlaceholder",
									"e.g. Q4 launch",
								)}
							/>
						</div>
						{picked.length > 0 && (
							<ul
								className="flex flex-wrap gap-1.5"
								aria-label={t("crew.newRoomDialog.members", "Members")}
							>
								{picked.map((principal) => (
									<li
										key={principal.id}
										className="inline-flex h-7 items-center gap-1 rounded-full bg-muted pr-1 pl-2.5 text-xs font-medium"
									>
										{principal.label}
										<button
											type="button"
											onClick={() => toggle(principal)}
											aria-label={t(
												"crew.newRoomDialog.remove",
												"Remove {{name}}",
												{
													name: principal.label,
												},
											)}
											className="inline-flex size-5 items-center justify-center rounded-full text-muted-foreground hover:bg-background hover:text-foreground"
										>
											<XIcon className="size-3" aria-hidden />
										</button>
									</li>
								))}
							</ul>
						)}
						<PrincipalPicker
							source={source}
							selected={selected}
							multiple
							full={picked.length >= MAX_OTHERS}
							serverRule={{}}
							unavailable={encrypted ? reason : undefined}
							onPick={toggle}
						/>
						{canSeal && (
							<div className="flex items-start gap-3 rounded-lg border p-3">
								<Switch
									id={sealedId}
									checked={sealed}
									onCheckedChange={setSealed}
									disabled={create.isPending || (!sealed && !sealable)}
									className="mt-0.5"
								/>
								<div className="flex min-w-0 flex-col gap-0.5">
									<Label htmlFor={sealedId} className="gap-1.5">
										<LockIcon className="size-3.5" aria-hidden />
										{t(
											"crew.newRoomDialog.encryptionLabel",
											"End-to-end encrypted",
										)}
									</Label>
									<p className="text-xs text-muted-foreground">
										{t(
											"crew.newRoomDialog.encryptionMeaning",
											"Only the members' devices can read it, the server can't: attachments and cards the server posts are limited, and each person reads only what is sent after they join on a device that has published its key.",
										)}
									</p>
									<SealingState
										blocked={blocked}
										reason={reason}
										checking={sealing.checking}
										failed={sealing.failed}
										urgent={encrypted}
									/>
								</div>
							</div>
						)}
						{invites > 0 && (
							<p className="text-xs text-muted-foreground">
								{t(
									"crew.newRoomDialog.invitesNote",
									"{{count}} will join once they accept; bots of other people need their owner to accept and set their limits.",
									{ count: invites },
								)}
							</p>
						)}
						{picked.length >= MAX_OTHERS && (
							<p className="text-xs text-muted-foreground">
								{t(
									"crew.newRoomDialog.full",
									"A room holds up to {{count}} members.",
									{
										count: MAX_CONVERSATION_MEMBERS,
									},
								)}
							</p>
						)}
						{mixesLocal && (
							<p aria-live="polite" className="text-xs text-muted-foreground">
								{t(
									"local.mixedRoom",
									"Bots on this device can only talk with you and your other bots on this device.",
								)}
							</p>
						)}
					</div>
				)}
			</div>
			{create.isError && (
				<p role="alert" className="text-sm text-destructive">
					{encrypted
						? encryptedRoomError(create.error, failed, t)
						: apiErrorMessage(create.error, failed)}
				</p>
			)}
			<DialogFooter>
				<Button
					type="button"
					variant="outline"
					disabled={create.isPending}
					onClick={settingUp ? () => setSettingUp(false) : onCancel}
				>
					{settingUp
						? t("roomLimits.setup.back", "Back")
						: t("crew.cancel", "Cancel")}
				</Button>
				<Button
					type="submit"
					disabled={
						picked.length === 0 ||
						mixesLocal ||
						(encrypted && !sealable) ||
						create.isPending
					}
				>
					{ownBots.length > 0 && !settingUp
						? t("roomLimits.setup.continue", "Continue to bot setup")
						: invites > 0
							? t("crew.newRoomDialog.createInvite", "Create room and invite")
							: t("crew.newRoomDialog.create", "Create room")}
				</Button>
			</DialogFooter>
		</form>
	);
}

export function NewRoomDialog({
	open,
	onOpenChange,
	source,
	onCreated,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	source: PickerSource;
	onCreated: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[85dvh] flex-col overflow-hidden sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t("crew.newRoomDialog.title", "New room")}</DialogTitle>
					<DialogDescription>
						{t(
							"crew.newRoomDialog.description",
							"Bring people and your bots together in one conversation.",
						)}
					</DialogDescription>
				</DialogHeader>
				{open && (
					<NewRoomForm
						source={source}
						onCancel={() => onOpenChange(false)}
						onCreated={(conversationId) => {
							onOpenChange(false);
							onCreated(conversationId);
						}}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
