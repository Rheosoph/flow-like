"use client";

import { useTranslation } from "@flow-like/locales";
import { CopyIcon, LinkIcon, Trash2Icon } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import {
	useConversationLinks,
	useCreateConversationLink,
	useRevokeConversationLink,
} from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import { getRoomInviteLink } from "../../lib/invite-link";
import { useBackend } from "../../state/backend-state";
import type {
	IConversation,
	IConversationLink,
} from "../../state/backend-state/inbox-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { isSelfPrincipal } from "./crew-model";

const EXPIRY_HOURS = [0, 24, 24 * 7, 24 * 30] as const;
const MAX_USES = [0, 1, 5, 10, 25, 100] as const;

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

function formatUnit(value: number, unit: "day" | "hour" | "minute"): string {
	return new Intl.NumberFormat(undefined, {
		style: "unit",
		unit,
		unitDisplay: "long",
	}).format(value);
}

function formatRemaining(ms: number): string {
	if (ms >= DAY_MS) return formatUnit(Math.floor(ms / DAY_MS), "day");
	if (ms >= HOUR_MS) return formatUnit(Math.floor(ms / HOUR_MS), "hour");
	return formatUnit(Math.max(1, Math.ceil(ms / 60_000)), "minute");
}

function useLinkMeta() {
	const { t } = useTranslation("bots");
	return (link: IConversationLink, now = Date.now()): string => {
		const parts: string[] = [];
		const usedUp = link.max_uses > 0 && link.count_joined >= link.max_uses;
		parts.push(
			link.max_uses > 0
				? t("roomLinks.usesOf", "{{count}} of {{max}} uses", {
						count: link.count_joined,
						max: link.max_uses,
					})
				: t("roomLinks.uses", "{{count}} uses", { count: link.count_joined }),
		);
		if (usedUp) parts.push(t("roomLinks.usedUp", "Used up"));
		if (link.expires_at) {
			const left = new Date(link.expires_at).getTime() - now;
			parts.push(
				left <= 0
					? t("roomLinks.expired", "Expired")
					: t("roomLinks.expiresIn", "Expires in {{time}}", {
							time: formatRemaining(left),
						}),
			);
		}
		return parts.join(" · ");
	};
}

function useCopyLink() {
	const { t } = useTranslation("bots");
	const profile = useBackend().profile;
	return async (token: string, successMessage: string) => {
		try {
			await navigator.clipboard.writeText(getRoomInviteLink(token, profile));
			toast.success(successMessage);
		} catch {
			toast.error(t("roomLinks.copyFailed", "Couldn't copy the link."));
		}
	};
}

function CreateLink({ conversationId }: Readonly<{ conversationId: string }>) {
	const { t } = useTranslation("bots");
	const create = useCreateConversationLink();
	const copy = useCopyLink();
	const [expiry, setExpiry] = useState<number>(0);
	const [maxUses, setMaxUses] = useState<number>(0);
	const expiryLabel = (hours: number) =>
		hours === 0
			? t("roomLinks.never", "Never")
			: formatUnit(
					hours % 24 === 0 ? hours / 24 : hours,
					hours % 24 === 0 ? "day" : "hour",
				);
	const submit = () =>
		create
			.mutateAsync({
				conversationId,
				request: {
					max_uses: maxUses > 0 ? maxUses : null,
					expires_in_hours: expiry > 0 ? expiry : null,
				},
			})
			.then((link) =>
				copy(link.token, t("roomLinks.created", "Link created and copied")),
			)
			.catch((error: unknown) =>
				toast.error(
					apiErrorMessage(
						error,
						t("roomLinks.createFailed", "Couldn't create the link."),
					),
				),
			);
	return (
		<div className="flex flex-wrap items-end gap-2">
			<div className="flex min-w-24 flex-1 flex-col gap-1 text-xs text-muted-foreground">
				<span aria-hidden>{t("roomLinks.expiry", "Expires")}</span>
				<Select
					value={String(expiry)}
					onValueChange={(value) => setExpiry(Number(value))}
				>
					<SelectTrigger
						size="sm"
						className="w-full"
						aria-label={t("roomLinks.expiry", "Expires")}
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{EXPIRY_HOURS.map((hours) => (
							<SelectItem key={hours} value={String(hours)}>
								{expiryLabel(hours)}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
			<div className="flex min-w-24 flex-1 flex-col gap-1 text-xs text-muted-foreground">
				<span aria-hidden>{t("roomLinks.maxUses", "Max uses")}</span>
				<Select
					value={String(maxUses)}
					onValueChange={(value) => setMaxUses(Number(value))}
				>
					<SelectTrigger
						size="sm"
						className="w-full"
						aria-label={t("roomLinks.maxUses", "Max uses")}
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{MAX_USES.map((uses) => (
							<SelectItem key={uses} value={String(uses)}>
								{uses === 0 ? t("roomLinks.noLimit", "No limit") : uses}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
			<Button
				type="button"
				size="sm"
				disabled={create.isPending}
				onClick={() => void submit()}
			>
				<LinkIcon aria-hidden />
				{t("roomLinks.create", "Create link")}
			</Button>
		</div>
	);
}

function LinkRow({
	link,
	canRevoke,
	onRevoke,
}: Readonly<{
	link: IConversationLink;
	canRevoke: boolean;
	onRevoke: (link: IConversationLink) => void;
}>) {
	const { t } = useTranslation("bots");
	const meta = useLinkMeta();
	const copy = useCopyLink();
	return (
		<li className="flex items-center gap-2 py-1.5">
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate text-sm font-medium">
					{link.name?.trim() || t("roomLinks.defaultName", "Invite link")}
				</span>
				<span className="truncate text-xs text-muted-foreground">
					{meta(link)}
				</span>
			</span>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				aria-label={t("roomLinks.copy", "Copy link")}
				onClick={() =>
					void copy(link.token, t("roomLinks.copied", "Link copied"))
				}
			>
				<CopyIcon aria-hidden />
			</Button>
			{canRevoke && (
				<Button
					type="button"
					variant="ghost"
					size="icon"
					aria-label={t("roomLinks.revoke", "Revoke link")}
					onClick={() => onRevoke(link)}
				>
					<Trash2Icon aria-hidden />
				</Button>
			)}
		</li>
	);
}

function RevokeDialog({
	conversationId,
	link,
	onClose,
}: Readonly<{
	conversationId: string;
	link: IConversationLink | null;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const revoke = useRevokeConversationLink();
	return (
		<AlertDialog
			open={link !== null}
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("roomLinks.revokeTitle", "Revoke this link?")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"roomLinks.revokeDescription",
							"Nobody can join with it afterwards. People who already joined stay in the room.",
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>
						{t("roomLinks.keep", "Keep link")}
					</AlertDialogCancel>
					<AlertDialogAction
						disabled={revoke.isPending}
						onClick={() => {
							if (!link) return;
							revoke
								.mutateAsync({ conversationId, linkId: link.id })
								.then(() =>
									toast.success(t("roomLinks.revoked", "Link revoked")),
								)
								.catch((error: unknown) =>
									toast.error(
										apiErrorMessage(
											error,
											t("roomLinks.revokeFailed", "Couldn't revoke the link."),
										),
									),
								);
						}}
					>
						{t("roomLinks.revoke", "Revoke link")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

/** Shareable join links for a plain group room; encrypted rooms get none yet. */
export function RoomInviteLinks({
	conversation,
	me,
}: Readonly<{ conversation: IConversation; me?: string | null }>) {
	const { t } = useTranslation("bots");
	const encrypted = conversation.encryption === "e2e";
	const links = useConversationLinks(conversation.id, !encrypted);
	const [revoking, setRevoking] = useState<IConversationLink | null>(null);
	const isCreator = isSelfPrincipal(conversation.created_by, me);
	return (
		<section className="flex flex-col gap-2">
			<h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
				{t("roomLinks.title", "Invite link")}
			</h3>
			{encrypted ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"roomLinks.encrypted",
						"Encrypted rooms can't have invite links yet.",
					)}
				</p>
			) : (
				<>
					<CreateLink conversationId={conversation.id} />
					{links.isError && (
						<p className="text-xs text-destructive">
							{apiErrorMessage(
								links.error,
								t("roomLinks.loadFailed", "Couldn't load the links."),
							)}
						</p>
					)}
					{(links.data?.length ?? 0) > 0 && (
						<ul className="flex flex-col">
							{links.data?.map((link) => (
								<LinkRow
									key={link.id}
									link={link}
									canRevoke={isCreator || isSelfPrincipal(link.created_by, me)}
									onRevoke={setRevoking}
								/>
							))}
						</ul>
					)}
					<RevokeDialog
						conversationId={conversation.id}
						link={revoking}
						onClose={() => setRevoking(null)}
					/>
				</>
			)}
		</section>
	);
}
