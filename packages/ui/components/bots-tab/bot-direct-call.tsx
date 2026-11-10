"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { PhoneIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useBotCallSpeech } from "../../hooks/use-bot-call-speech";
import { useConversationActivity } from "../../hooks/use-conversation-activity";
import { useConversationCall } from "../../hooks/use-conversation-call";
import {
	type ConversationRoom,
	useConversationRoom,
} from "../../hooks/use-conversation-room";
import { useInboxEntries } from "../../hooks/use-inbox";
import { useSpeechModelState } from "../../hooks/use-speech-model";
import { useBackend } from "../../state/backend-state";
import { type IBot, LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { botConversation } from "../../state/global-chat/bot-history";
import { Button } from "../ui/button";
import { CallSpeechSetup } from "./call-speech-setup";
import { ConversationActivityLine } from "./conversation-activity-line";
import {
	CallButton,
	ConversationCallBar,
	useCallErrorText,
} from "./conversation-call";
import { foldInboxEntries } from "./fold-entries";
import { callMentions } from "./mentions";
import { type CrewDirectory, useCrewDirectory } from "./use-crew-directory";

const EMPTY_CONVERSATION: IConversation = {
	id: "",
	kind: "direct",
	title: null,
	encryption: "plain",
	created_by: "",
	created_at: "",
	last_activity_at: "",
	members: [],
	unread_count: 0,
	last_entry: null,
};

/** Keeps direct calls in the bot's existing inbox, alongside its text chat. */
export function BotDirectCall({
	bot,
	children,
}: {
	bot?: IBot;
	children: (controls: { button: ReactNode; bar: ReactNode }) => ReactNode;
}) {
	const { t } = useTranslation("bots");
	const inbox = useBackend().inboxState;
	const me = bot ? (bot.local ? LOCAL_OWNER_ID : bot.owner_id) : null;
	const conversationQuery = useQuery({
		queryKey: ["bot-call-conversation", bot?.id, me],
		enabled: Boolean(bot && inbox),
		queryFn: async () => {
			if (!bot || !inbox) throw new Error("Bot calls need an inbox");
			return inbox.getConversation(await botConversation(inbox, bot.id));
		},
	});
	const conversation = conversationQuery.data ?? EMPTY_CONVERSATION;
	const room = useConversationRoom(
		conversation,
		me,
		Boolean(conversationQuery.data),
	);
	const history = useInboxEntries(conversationQuery.data?.id);
	const pages = history.data?.pages;
	const entries = useMemo(
		() => pages?.flatMap((page) => page.entries) ?? [],
		[pages],
	);
	const items = useMemo(() => foldInboxEntries(entries, me), [entries, me]);
	const bots = useMemo(() => (bot ? [bot] : []), [bot]);
	const principals = useMemo(
		() => conversation.members.map((member) => member.principal_id),
		[conversation.members],
	);
	const directory = useCrewDirectory(principals, { me, bots });
	const speech = useSpeechModelState(Boolean(bot && inbox), bot?.stt_model);
	const stt = speech.choice;
	const [setupOpen, setSetupOpen] = useState(false);
	const listen = useMemo(
		() => ({
			stt,
			hasBots: Boolean(bot),
			mentionsIn: (spoken: string) =>
				callMentions(
					spoken,
					bot ? [{ id: bot.id, name: bot.name ?? "", kind: "bot" }] : [],
					"direct",
				),
		}),
		[stt, bot],
	);
	const starts = useMemo(
		() =>
			new Map(
				items.flatMap((item) =>
					item.type === "call" ? [[item.id, item.time] as const] : [],
				),
			),
		[items],
	);
	const call = useConversationCall(conversation, me, room, starts, listen);
	useBotCallSpeech({
		conversation,
		entries,
		items,
		historyReady: pages !== undefined,
		directory,
		call,
	});
	const errorText = useCallErrorText();
	useEffect(() => {
		if (call.error) toast.error(errorText(call.error));
	}, [call.error, errorText]);
	const listenRequested = useRef(false);
	useEffect(() => {
		if (call.error) listenRequested.current = false;
	}, [call.error]);
	const start = useCallback(async () => {
		listenRequested.current = true;
		try {
			await call.start();
		} catch (cause) {
			listenRequested.current = false;
			throw cause;
		}
	}, [call.start]);
	const join = useCallback(async () => {
		listenRequested.current = true;
		try {
			await call.join();
		} catch (cause) {
			listenRequested.current = false;
			throw cause;
		}
	}, [call.join]);
	const leave = useCallback(async () => {
		listenRequested.current = false;
		await call.leave();
	}, [call.leave]);
	useEffect(() => {
		if (!call.joined || !call.canListen || !listenRequested.current) return;
		listenRequested.current = false;
		call.setListening(true);
	}, [call.joined, call.canListen, call.setListening]);
	const needsSpeech = speech.settled && !stt;
	const button =
		bot && inbox ? (
			needsSpeech || setupOpen ? (
				<CallSpeechSetup
					bot={bot}
					name={directory(bot.id).name}
					speech={speech}
					open={setupOpen}
					onOpenChange={setSetupOpen}
					canStart={call.available}
					onStart={() => void start()}
				/>
			) : call.available ? (
				<CallButton call={{ ...call, start }} />
			) : (
				<Button
					type="button"
					variant="ghost"
					size="icon"
					disabled={!conversationQuery.isError}
					onClick={() => void conversationQuery.refetch()}
					aria-label={
						conversationQuery.isError
							? t("crew.call.retrySetup", "Retry call setup")
							: t("crew.call.start", "Start a call")
					}
					title={
						conversationQuery.isError
							? t("crew.call.error.failed", "The call didn't start. Try again.")
							: t("crew.call.preparing", "Preparing the call")
					}
				>
					<PhoneIcon aria-hidden />
				</Button>
			)
		) : null;
	return children({
		button,
		bar: call.live ? (
			<>
				<ConversationCallBar
					call={{ ...call, join, leave }}
					directory={directory}
					me={me}
				/>
				{call.joined && (
					<BotCallActivity
						conversation={conversation}
						me={me}
						newestEntryKey={pages?.[0]?.tail ?? entries[0]?.cursor}
						room={room}
						directory={directory}
					/>
				)}
			</>
		) : null,
	});
}

function BotCallActivity({
	conversation,
	me,
	newestEntryKey,
	room,
	directory,
}: {
	conversation: IConversation;
	me: string | null;
	newestEntryKey: string | undefined;
	room: ConversationRoom | undefined;
	directory: CrewDirectory;
}) {
	const activity = useConversationActivity(
		conversation,
		me,
		newestEntryKey,
		room,
	);
	return <ConversationActivityLine {...activity} directory={directory} />;
}
