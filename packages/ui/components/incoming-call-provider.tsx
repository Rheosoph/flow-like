"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { PhoneIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useConversationRoom } from "../hooks/use-conversation-room";
import { useUserAvailability } from "../hooks/use-user-availability";
import { shouldPresentUserNotification } from "../lib/availability-notifications";
import { deriveLiveCall } from "../lib/realtime/conversation-call";
import { watchConversation } from "../lib/realtime/conversation-feed";
import {
	registerIncomingCallScope,
	requestIncomingCallJoin,
} from "../lib/realtime/incoming-call-join";
import {
	type IncomingCallNotice,
	incomingCallEnded,
	incomingCallNotice,
	incomingCallResolutionKey,
	incomingCallSender,
	isIncomingCallResolved,
	resolveIncomingCall,
	subscribeIncomingCallResolutions,
} from "../lib/realtime/incoming-call-notification";
import { useBackend } from "../state/backend-state";
import type { IConversation } from "../state/backend-state/inbox-state";
import { IdentityAvatar } from "./identity/identity-avatar";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "./ui/alert-dialog";
import { Button } from "./ui/button";

interface IncomingCallProviderProps {
	enabled?: boolean;
	scope: string;
	userId: string;
}

/** Mounted with the account shell so calls can reach readers outside their conversation. */
export function IncomingCallProvider({
	enabled = true,
	scope,
	userId,
}: Readonly<IncomingCallProviderProps>) {
	return enabled && scope && userId ? (
		<IncomingCallSession
			key={JSON.stringify([scope, userId])}
			scope={scope}
			userId={userId}
		/>
	) : null;
}

function IncomingCallSession({
	scope,
	userId,
}: Readonly<IncomingCallProviderProps>) {
	const backend = useBackend();
	const availability = useUserAvailability();
	const localAvailability = useUserAvailability({ local: true });
	const [now, setNow] = useState(Date.now);
	const [, setResolutionVersion] = useState(0);
	const [ignored, setIgnored] = useState<ReadonlySet<string>>(() => new Set());
	const resolutionKey = incomingCallResolutionKey(scope, userId);
	useEffect(() => registerIncomingCallScope(backend, scope), [backend, scope]);
	const notifications = useQuery({
		queryKey: [
			backend.userState.listNotifications.name,
			"incoming-calls",
			scope,
			userId,
		],
		queryFn: () => backend.userState.listNotifications(false, "SYSTEM", 0, 50),
		enabled: Boolean(backend.inboxState),
		refetchInterval: 10_000,
		refetchIntervalInBackground: true,
	});
	useEffect(() => {
		const update = () => setNow(Date.now());
		const stop = subscribeIncomingCallResolutions(resolutionKey, () => {
			setResolutionVersion((version) => version + 1);
		});
		const timer = setInterval(update, 1_000);
		return () => {
			stop();
			clearInterval(timer);
		};
	}, [resolutionKey]);
	const notice = (notifications.data ?? [])
		.map((notification) => incomingCallNotice(notification, userId, now))
		.find(
			(candidate) =>
				candidate &&
				!ignored.has(candidate.callId) &&
				!isIncomingCallResolved(resolutionKey, candidate.callId),
		);
	const resolve = useCallback(
		(callId: string) => {
			resolveIncomingCall(resolutionKey, callId);
			setNow(Date.now());
		},
		[resolutionKey],
	);
	const ignore = useCallback((callId: string) => {
		setIgnored((current) => new Set([...current, callId]));
	}, []);
	return notice &&
		!availability.isPending &&
		shouldPresentUserNotification(
			availability.data,
			localAvailability.data,
			"call",
			now,
		) ? (
		<CheckIncomingCall
			key={notice.callId}
			notice={notice}
			scope={scope}
			userId={userId}
			resolve={resolve}
			ignore={ignore}
		/>
	) : null;
}

interface CheckCallProps {
	notice: IncomingCallNotice;
	scope: string;
	userId: string;
	resolve: (callId: string) => void;
	ignore: (callId: string) => void;
}

function CheckIncomingCall({
	notice,
	scope,
	userId,
	resolve,
	ignore,
}: Readonly<CheckCallProps>) {
	const inbox = useBackend().inboxState;
	const dismiss = useCallback(
		() => resolve(notice.callId),
		[resolve, notice.callId],
	);
	const checked = useQuery({
		queryKey: [
			"inbox",
			"incoming-call",
			scope,
			userId,
			notice.conversationId,
			notice.callId,
		],
		queryFn: async () => {
			if (!inbox) return null;
			const [conversation, page, blocks] = await Promise.all([
				inbox.getConversation(notice.conversationId),
				inbox.listEntries(notice.conversationId, { limit: 100 }),
				inbox.listBlocks?.() ?? Promise.resolve([]),
			]);
			const sender = incomingCallSender(
				notice,
				conversation,
				page.entries,
				userId,
			);
			return sender && !blocks.some((block) => block.principal_id === sender)
				? { conversation, sender, cursor: page.entries[0]?.cursor }
				: null;
		},
		refetchInterval: 3_000,
		refetchIntervalInBackground: true,
		retry: false,
	});
	useEffect(() => {
		if (checked.data === null) ignore(notice.callId);
	}, [checked.data, notice.callId, ignore]);
	useEffect(() => {
		if (!checked.data) return;
		return watchConversation(
			{ conversationId: notice.conversationId, after: checked.data.cursor },
			(entries) => {
				if (incomingCallEnded(entries, notice.callId)) dismiss();
			},
		);
	}, [checked.data, notice.conversationId, notice.callId, dismiss]);
	return checked.data && !checked.isError ? (
		<LiveIncomingCall
			{...{ notice, scope, userId, resolve, ignore }}
			conversation={checked.data.conversation}
			sender={checked.data.sender}
		/>
	) : null;
}

function LiveIncomingCall({
	notice,
	scope,
	userId,
	resolve,
	conversation,
	sender,
}: Readonly<CheckCallProps & { conversation: IConversation; sender: string }>) {
	const { t } = useTranslation("bots");
	const router = useRouter();
	const room = useConversationRoom(conversation, userId);
	const [live, setLive] = useState(false);
	const members = useMemo(
		() =>
			new Set(
				conversation.members
					.filter((member) => member.kind === "user")
					.map((member) => member.principal_id),
			),
		[conversation.members],
	);
	const dismiss = useCallback(
		() => resolve(notice.callId),
		[resolve, notice.callId],
	);
	useEffect(() => {
		setLive(false);
		if (!room) return;
		const update = () => {
			const current = deriveLiveCall(
				room.awareness.getStates(),
				room.awareness.clientID,
				members,
			);
			if (current?.participants.some((person) => person.sub === userId)) {
				dismiss();
				return;
			}
			const present =
				current?.callId === notice.callId && current.participants.length > 0;
			setLive(present);
		};
		room.awareness.on("change", update);
		update();
		return () => {
			room.awareness.off("change", update);
		};
	}, [room, members, notice.callId, userId, dismiss]);
	if (!live) return null;
	const join = () => {
		requestIncomingCallJoin({
			scope,
			userId,
			conversationId: notice.conversationId,
			callId: notice.callId,
		});
		dismiss();
		router.push(
			`/bots?conversation=${encodeURIComponent(notice.conversationId)}`,
		);
	};
	return (
		<AlertDialog
			open
			onOpenChange={(open) => {
				if (!open) dismiss();
			}}
		>
			<AlertDialogContent className="sm:max-w-md">
				<AlertDialogHeader>
					<div className="mb-2 flex items-center gap-3">
						<IdentityAvatar subject={sender} size={56} />
						<div className="min-w-0">
							<AlertDialogTitle>
								{t("crew.call.incoming", "Incoming call")}
							</AlertDialogTitle>
							<p className="truncate text-sm font-medium">
								{notice.notification.title}
							</p>
						</div>
					</div>
					<AlertDialogDescription>
						{conversation.kind === "group" && conversation.title
							? t("crew.call.incomingRoom", "Join the call in {{room}}.", {
									room: conversation.title,
								})
							: t(
									"crew.call.incomingDescription",
									"Join the conversation call.",
								)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>
						{t("crew.call.dismiss", "Dismiss")}
					</AlertDialogCancel>
					<Button type="button" onClick={join}>
						<PhoneIcon aria-hidden />
						{t("crew.call.join", "Join")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
