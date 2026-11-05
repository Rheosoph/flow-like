"use client";

import { useTranslation } from "@flow-like/locales";
import { CalendarDaysIcon, SendIcon, UsersIcon } from "lucide-react";
import {
	type FormEvent,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useConversationCall } from "../../hooks/use-conversation-call";
import { useMeetingRoom } from "../../hooks/use-meeting-room";
import { useMeetingSession } from "../../hooks/use-meeting-session";
import { LocalPrincipalsProvider } from "../../hooks/use-user-lookup";
import { meetingError, meetingPhase } from "../../lib/meetings/helpers";
import type {
	MeetingClient,
	MeetingCredentials,
	MeetingPreview,
	MeetingSession,
} from "../../lib/meetings/types";
import type { IConversation } from "../../state/backend-state/inbox-state";
import {
	ConversationCallBar,
	useCallErrorText,
} from "../bots-tab/conversation-call";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Textarea } from "../ui/textarea";

export interface MeetingPageProps {
	meetingId: string;
	inviteToken?: string;
	client: MeetingClient;
	me?: string | null;
	displayName?: string;
	signalingServers?: string[];
	onSignIn?: () => Promise<void>;
}

/** The public join surface shares no room history, bot tools, or persistent room membership. */
export function MeetingPage(props: MeetingPageProps) {
	return (
		<MeetingPageContent
			key={`${props.meetingId}:${props.inviteToken ?? ""}`}
			{...props}
		/>
	);
}

function MeetingPageContent({
	meetingId,
	inviteToken,
	client,
	me,
	displayName,
	signalingServers,
	onSignIn,
}: MeetingPageProps) {
	const { t } = useTranslation("bots");
	const id = useId();
	const [preview, setPreview] = useState<MeetingPreview>();
	const [credentials, setCredentials] = useState<MeetingCredentials>();
	const [name, setName] = useState(displayName ?? "");
	const [error, setError] = useState<string>();
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState(false);
	const [now, setNow] = useState(Date.now);
	useEffect(() => {
		if (!me && !inviteToken) {
			setLoading(false);
			return;
		}
		let disposed = false;
		const load = async () => {
			try {
				const value = await client.preview(meetingId, inviteToken);
				if (!disposed) {
					setPreview(value);
					setError(undefined);
				}
			} catch (cause) {
				if (!disposed) setError(meetingError(cause));
			} finally {
				if (!disposed) setLoading(false);
			}
		};
		void load();
		const poll = setInterval(() => void load(), 15_000);
		const clock = setInterval(() => setNow(Date.now()), 1000);
		return () => {
			disposed = true;
			clearInterval(poll);
			clearInterval(clock);
		};
	}, [client, meetingId, inviteToken, me]);
	const join = async (guest: boolean) => {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		try {
			const result =
				guest && inviteToken
					? await client.joinGuest(meetingId, inviteToken, name.trim())
					: await client.join(meetingId);
			setCredentials(result);
		} catch (cause) {
			setError(meetingError(cause));
		} finally {
			setBusy(false);
		}
	};
	return (
		<main className="min-h-dvh bg-background text-foreground">
			<div className="mx-auto flex min-h-dvh max-w-7xl flex-col gap-6 px-4 py-8 sm:px-8">
				<header className="flex items-start gap-3">
					<span className="rounded-xl border bg-muted/40 p-3">
						<CalendarDaysIcon className="size-6" aria-hidden />
					</span>
					<div className="min-w-0">
						<p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
							{t("crew.meetings.brand", "Flow-Like meeting")}
						</p>
						<h1 className="mt-1 break-words text-2xl font-semibold tracking-tight">
							{preview?.title ?? t("crew.meetings.open", "Meetings")}
						</h1>
						{preview && (
							<p className="mt-1 text-sm text-muted-foreground">
								{new Date(preview.starts_at).toLocaleString()} –{" "}
								{new Date(preview.ends_at).toLocaleTimeString([], {
									hour: "2-digit",
									minute: "2-digit",
								})}
							</p>
						)}
					</div>
				</header>
				{credentials ? (
					<MeetingSessionView
						client={client}
						meetingId={meetingId}
						credentials={credentials}
						signalingServers={signalingServers}
						onLeave={() => setCredentials(undefined)}
					/>
				) : (
					<section className="mx-auto w-full max-w-md space-y-5 rounded-2xl border bg-card p-6 shadow-sm">
						{loading ? (
							<output>{t("crew.meetings.loading", "Loading meeting…")}</output>
						) : !preview ? (
							<p>
								{t(
									"crew.meetings.unavailable",
									"This meeting is unavailable. Open your invitation link, or sign in with an account that belongs to the room.",
								)}
							</p>
						) : meetingPhase(preview, now) === "ended" ? (
							<p>{t("crew.meetings.ended", "Meeting ended")}</p>
						) : meetingPhase(preview, now) === "upcoming" ? (
							<>
								<h2 className="font-medium">
									{t("crew.meetings.notYet", "Your meeting is scheduled")}
								</h2>
								<p className="text-sm text-muted-foreground">
									{t(
										"crew.meetings.opensSoon",
										"You can join 15 minutes before the start time. Keep this page open or come back then.",
									)}
								</p>
							</>
						) : (
							<>
								<h2 className="text-lg font-medium">
									{t("crew.meetings.ready", "Ready to join?")}
								</h2>
								<p className="text-sm text-muted-foreground">
									{t(
										"crew.meetings.mediaOff",
										"Your microphone and camera stay off until you join the call and choose what to share.",
									)}
								</p>
								{me && (
									<Button
										className="w-full"
										disabled={busy}
										onClick={() => void join(false)}
									>
										{t("crew.meetings.joinAccount", "Join with my account")}
									</Button>
								)}
								{inviteToken && preview.guests_allowed && (
									<form
										className="space-y-3"
										onSubmit={(event) => {
											event.preventDefault();
											if (name.trim()) void join(true);
										}}
									>
										<Label htmlFor={`${id}-name`}>
											{t("crew.meetings.yourName", "Your name")}
										</Label>
										<Input
											id={`${id}-name`}
											value={name}
											maxLength={80}
											required
											onChange={(event) => setName(event.target.value)}
											autoComplete="name"
										/>
										<Button
											className="w-full"
											variant={me ? "outline" : "default"}
											type="submit"
											disabled={busy || !name.trim()}
										>
											{t("crew.meetings.joinGuest", "Ask to join as a guest")}
										</Button>
										<p className="text-xs text-muted-foreground">
											{t(
												"crew.meetings.guestScope",
												"The host will admit you. Your access is limited to this meeting and expires when it ends.",
											)}
										</p>
									</form>
								)}
								{!me && !inviteToken && (
									<p className="text-sm text-muted-foreground">
										{t(
											"crew.meetings.needInvitation",
											"Ask the host for a guest invitation link, or sign in to join as a room member.",
										)}
									</p>
								)}
							</>
						)}
						{!me && onSignIn && (
							<Button
								variant="outline"
								className="w-full"
								disabled={busy}
								onClick={async () => {
									setBusy(true);
									try {
										await onSignIn();
									} catch (cause) {
										setError(meetingError(cause));
									} finally {
										setBusy(false);
									}
								}}
							>
								{t("crew.meetings.signIn", "Sign in to join with your account")}
							</Button>
						)}
						{error && (
							<p role="alert" className="text-sm text-destructive">
								{error}
							</p>
						)}
					</section>
				)}
			</div>
		</main>
	);
}

function MeetingSessionView({
	client,
	meetingId,
	credentials,
	signalingServers,
	onLeave,
}: {
	client: MeetingClient;
	meetingId: string;
	credentials: MeetingCredentials;
	signalingServers?: string[];
	onLeave: () => void;
}) {
	const { t } = useTranslation("bots");
	const { snapshot, active, error, terminal, refresh } = useMeetingSession(
		client,
		meetingId,
		credentials,
	);
	const [actionError, setActionError] = useState<string>();
	const [busy, setBusy] = useState(false);
	const leaveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	useEffect(() => {
		clearTimeout(leaveTimer.current);
		const leaveServer = () => {
			void client.leave(meetingId, credentials.session_token).catch(() => {});
		};
		window.addEventListener("pagehide", leaveServer);
		return () => {
			window.removeEventListener("pagehide", leaveServer);
			// A same-tick remount cancels this; React's development checks must not end a session.
			leaveTimer.current = setTimeout(leaveServer, 0);
		};
	}, [client, meetingId, credentials.session_token]);
	const action = async (fn: () => Promise<void>) => {
		setBusy(true);
		setActionError(undefined);
		try {
			await fn();
			await refresh();
		} catch (cause) {
			setActionError(meetingError(cause));
		} finally {
			setBusy(false);
		}
	};
	const ended =
		!!snapshot &&
		meetingPhase(snapshot.meeting, Date.parse(snapshot.server_time)) ===
			"ended";
	return (
		<>
			<div className="flex flex-wrap items-center justify-between gap-3">
				<p className="min-w-0 break-words text-sm text-muted-foreground">
					{snapshot?.me.display_name}
				</p>
				<div className="flex gap-2">
					{snapshot?.me.is_host && !ended && (
						<Button
							variant="destructive"
							disabled={busy}
							onClick={() =>
								void action(() =>
									client.end(meetingId, credentials.session_token),
								)
							}
						>
							{t("crew.meetings.endEveryone", "End meeting for everyone")}
						</Button>
					)}
					<Button variant="outline" disabled={busy} onClick={onLeave}>
						{t("crew.meetings.leave", "Leave meeting")}
					</Button>
				</div>
			</div>
			{(error || actionError) && (
				<p role="alert" className="text-sm text-destructive">
					{actionError ?? error}
				</p>
			)}
			{!snapshot ? (
				<output>
					{t("crew.meetings.connecting", "Connecting to the meeting…")}
				</output>
			) : terminal ||
				ended ||
				snapshot.me.status === "removed" ||
				snapshot.me.status === "left" ? (
				<div className="rounded-xl border p-6">
					<p>
						{snapshot.me.status === "removed"
							? t(
									"crew.meetings.removed",
									"The host declined or ended your participation.",
								)
							: t("crew.meetings.ended", "Meeting ended")}
					</p>
				</div>
			) : snapshot.me.status === "waiting" ? (
				<section className="mx-auto w-full max-w-md space-y-3 rounded-2xl border bg-card p-6 text-center">
					<UsersIcon
						className="mx-auto size-8 text-muted-foreground"
						aria-hidden
					/>
					<h2 className="text-lg font-medium">
						{t("crew.meetings.waiting", "Waiting for the host")}
					</h2>
					<output className="text-sm text-muted-foreground">
						{t(
							"crew.meetings.waitingHint",
							"Your request has been sent. This page updates when the host admits you.",
						)}
					</output>
				</section>
			) : (
				<>
					{snapshot.me.is_host && (
						<MeetingParticipants
							snapshot={snapshot}
							busy={busy}
							onAdmit={(id) =>
								void action(() =>
									client.admit(meetingId, id, credentials.session_token),
								)
							}
							onRemove={(id) =>
								void action(() =>
									client.remove(meetingId, id, credentials.session_token),
								)
							}
						/>
					)}
					{active ? (
						<LiveMeeting
							client={client}
							snapshot={snapshot}
							credentials={credentials}
							signalingServers={signalingServers}
							onSend={refresh}
						/>
					) : (
						<output className="rounded-xl border p-6 text-muted-foreground">
							{t(
								"crew.meetings.reconnecting",
								"Checking meeting access. Audio and video are paused until the connection is restored.",
							)}
						</output>
					)}
				</>
			)}
		</>
	);
}

export function MeetingParticipants({
	snapshot,
	busy,
	onAdmit,
	onRemove,
}: {
	snapshot: MeetingSession;
	busy: boolean;
	onAdmit: (id: string) => void;
	onRemove: (id: string) => void;
}) {
	const { t } = useTranslation("bots");
	const participants = snapshot.participants.filter(
		(p) =>
			p.id !== snapshot.me.id &&
			(p.status === "waiting" || p.status === "admitted"),
	);
	if (!participants.length) return null;
	return (
		<section className="rounded-xl border p-4">
			<h2 className="mb-3 text-sm font-medium">
				{t("crew.meetings.participants", "Participants")}
			</h2>
			<ul className="space-y-2">
				{participants.map((person) => (
					<li key={person.id} className="flex flex-wrap items-center gap-2">
						<span className="min-w-0 flex-1 truncate text-sm">
							{person.display_name}
							{person.guest && (
								<span className="ml-2 text-xs text-muted-foreground">
									{t("crew.meetings.guest", "Guest")}
								</span>
							)}
						</span>
						{person.status === "waiting" && (
							<Button
								size="sm"
								disabled={busy}
								onClick={() => onAdmit(person.id)}
								aria-label={t("crew.meetings.admitName", "Admit {{name}}", {
									name: person.display_name,
								})}
							>
								{t("crew.meetings.admit", "Admit")}
							</Button>
						)}
						<Button
							size="sm"
							variant="ghost"
							disabled={busy}
							onClick={() => onRemove(person.id)}
							aria-label={t("crew.meetings.removeName", "Remove {{name}}", {
								name: person.display_name,
							})}
						>
							{person.status === "waiting"
								? t("crew.meetings.decline", "Decline")
								: t("crew.meetings.remove", "Remove")}
						</Button>
					</li>
				))}
			</ul>
		</section>
	);
}

function LiveMeeting({
	client,
	snapshot,
	credentials,
	signalingServers,
	onSend,
}: {
	client: MeetingClient;
	snapshot: MeetingSession;
	credentials: MeetingCredentials;
	signalingServers?: string[];
	onSend: () => Promise<void>;
}) {
	const { t } = useTranslation("bots");
	const { room, error } = useMeetingRoom({
		client,
		meetingId: snapshot.meeting.id,
		token: credentials.session_token,
		participantId: snapshot.me.id,
		active: true,
		roomEpoch: snapshot.room_epoch,
		signalingServers,
	});
	const memberKey = JSON.stringify(
		snapshot.participants
			.filter((p) => p.status === "admitted")
			.map((p) => p.id)
			.sort(),
	);
	const conversation = useMemo<IConversation>(
		() => ({
			id: `meeting-${snapshot.meeting.id}`,
			kind: "group",
			title: snapshot.meeting.title,
			encryption: "plain",
			created_by: snapshot.meeting.host_id,
			created_at: snapshot.meeting.starts_at,
			last_activity_at: snapshot.meeting.starts_at,
			unread_count: 0,
			last_entry: null,
			members: (JSON.parse(memberKey) as string[]).map((id) => ({
				principal_id: id,
				kind: "user",
				added_by: null,
				joined_at: snapshot.meeting.starts_at,
			})),
		}),
		[
			memberKey,
			snapshot.meeting.id,
			snapshot.meeting.title,
			snapshot.meeting.host_id,
			snapshot.meeting.starts_at,
		],
	);
	const call = useConversationCall(
		conversation,
		snapshot.me.id,
		room,
		undefined,
		undefined,
		{ persistCallEvents: false, stableCallId: snapshot.meeting.id },
	);
	const callError = useCallErrorText();
	const directory = useCallback(
		(id: string) => ({
			id,
			name:
				snapshot.participants.find((p) => p.id === id)?.display_name ??
				t("crew.meetings.participant", "Participant"),
			isMe: id === snapshot.me.id,
			isBot: false,
			isPending: false,
			isResolved: snapshot.participants.some(
				(participant) => participant.id === id,
			),
		}),
		[snapshot.participants, snapshot.me.id, t],
	);
	const resolve = useCallback(
		(id: string) => {
			const participant = snapshot.participants.find((p) => p.id === id);
			return participant
				? {
						id,
						name: participant.display_name,
						created_at: snapshot.meeting.starts_at,
					}
				: null;
		},
		[snapshot.participants, snapshot.meeting.starts_at],
	);
	return (
		<LocalPrincipalsProvider value={resolve}>
			<div className="grid min-h-0 flex-1 gap-5 lg:grid-cols-[minmax(0,1fr)_320px]">
				<section className="min-w-0 overflow-hidden rounded-2xl border bg-card">
					<div className="flex items-center justify-between gap-3 border-b p-4">
						<h2 className="text-base font-medium">
							{t("crew.meetings.call", "Meeting call")}
						</h2>
						{!call.joined && (
							<Button
								disabled={!call.available}
								onClick={() => void call.start()}
							>
								{t("crew.meetings.joinCall", "Join audio and video")}
							</Button>
						)}
					</div>
					{error && (
						<p role="alert" className="p-4 text-sm text-destructive">
							{error}
						</p>
					)}
					{call.error && (
						<p role="alert" className="p-4 text-sm text-destructive">
							{callError(call.error)}
						</p>
					)}
					<ConversationCallBar
						call={call}
						me={snapshot.me.id}
						directory={directory}
					/>
					{!call.live && (
						<div className="flex min-h-60 flex-col items-center justify-center gap-3 px-6 text-center text-muted-foreground">
							<UsersIcon className="size-10" aria-hidden />
							<p>
								{call.available
									? t(
											"crew.meetings.callReady",
											"Join the call when you are ready. You can enable your camera or share a screen from the call controls.",
										)
									: t(
											"crew.meetings.mediaConnecting",
											"Connecting audio and video…",
										)}
							</p>
						</div>
					)}
				</section>
				<MeetingChat
					client={client}
					snapshot={snapshot}
					token={credentials.session_token}
					onSend={onSend}
				/>
			</div>
		</LocalPrincipalsProvider>
	);
}

function MeetingChat({
	client,
	snapshot,
	token,
	onSend,
}: {
	client: MeetingClient;
	snapshot: MeetingSession;
	token: string;
	onSend: () => Promise<void>;
}) {
	const { t } = useTranslation("bots");
	const [draft, setDraft] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const bottom = useRef<HTMLDivElement>(null);
	const lastMessage = snapshot.messages.at(-1)?.id;
	useEffect(() => {
		if (lastMessage) bottom.current?.scrollIntoView?.({ block: "nearest" });
	}, [lastMessage]);
	const submit = async (event: FormEvent) => {
		event.preventDefault();
		const body = draft.trim();
		if (!body || busy) return;
		setBusy(true);
		setError(undefined);
		try {
			await client.send(snapshot.meeting.id, token, body);
			setDraft("");
			await onSend();
		} catch (cause) {
			setError(meetingError(cause));
		} finally {
			setBusy(false);
		}
	};
	return (
		<section className="flex min-h-80 flex-col rounded-2xl border bg-card">
			<h2 className="border-b p-4 text-base font-medium">
				{t("crew.meetings.chat", "Meeting chat")}
			</h2>
			<div
				className="max-h-[50dvh] min-h-40 flex-1 space-y-4 overflow-y-auto p-4"
				role="log"
				aria-label={t("crew.meetings.chat", "Meeting chat")}
				aria-relevant="additions text"
			>
				{snapshot.messages.map((message) => (
					<div key={message.id}>
						<div className="flex items-baseline justify-between gap-2">
							<span className="text-xs font-medium">
								{message.display_name}
							</span>
							<time
								dateTime={message.created_at}
								className="text-[10px] text-muted-foreground"
							>
								{new Date(message.created_at).toLocaleTimeString([], {
									hour: "2-digit",
									minute: "2-digit",
								})}
							</time>
						</div>
						<p className="mt-1 whitespace-pre-wrap break-words text-sm">
							{message.body}
						</p>
					</div>
				))}
				<div ref={bottom} />
			</div>
			<form onSubmit={submit} className="space-y-2 border-t p-3">
				<Textarea
					value={draft}
					onChange={(event) => setDraft(event.target.value)}
					maxLength={2000}
					rows={2}
					aria-label={t("crew.meetings.message", "Message")}
					placeholder={t("crew.meetings.writeMessage", "Write a message…")}
				/>
				<div className="flex items-center justify-end gap-2">
					{error && (
						<p role="alert" className="flex-1 text-xs text-destructive">
							{error}
						</p>
					)}
					<Button type="submit" size="sm" disabled={busy || !draft.trim()}>
						<SendIcon aria-hidden />
						{t("crew.meetings.send", "Send")}
					</Button>
				</div>
			</form>
		</section>
	);
}
