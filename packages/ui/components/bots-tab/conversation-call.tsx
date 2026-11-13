"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AudioLinesIcon,
	HandIcon,
	MicIcon,
	MicOffIcon,
	MonitorOffIcon,
	MonitorUpIcon,
	PhoneIcon,
	PhoneOffIcon,
	VideoIcon,
	VideoOffIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type {
	CallError,
	ConversationCallState,
} from "../../hooks/use-conversation-call";
import {
	formatClock,
	splitCallDuration,
} from "../../lib/realtime/conversation-call";
import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import { Switch } from "../ui/switch";
import { CallVideoTile } from "./call-video-tile";
import type { ConversationCall } from "./fold-entries";
import type { CrewDirectory } from "./use-crew-directory";

/** What a call line needs from the call: which call is live and how to join it. */
export interface ThreadCall {
	liveCallId: string | null;
	joined: boolean;
	onJoin: () => void;
}

export function useCallDuration() {
	const { t } = useTranslation("bots");
	return useCallback(
		(ms: number): string => {
			const { hours, minutes } = splitCallDuration(ms);
			if (hours > 0) {
				return t("crew.call.durationHours", "{{hours}} h {{minutes}} min", {
					hours,
					minutes,
				});
			}
			if (minutes > 0) {
				return t("crew.call.durationMinutes", "{{minutes}} min", { minutes });
			}
			return t("crew.call.durationShort", "under 1 min");
		},
		[t],
	);
}

export function useCallErrorText() {
	const { t } = useTranslation("bots");
	return useCallback(
		(error: CallError): string =>
			({
				denied: t(
					"crew.call.error.denied",
					"Microphone access is blocked. Allow it in your browser settings to join the call.",
				),
				unavailable: t(
					"crew.call.error.unavailable",
					"No microphone was found, or another app is using it.",
				),
				full: t("crew.call.error.full", "This call is full."),
				failed: t(
					"crew.call.error.failed",
					"The call didn't start. Try again.",
				),
				transcription: t(
					"crew.call.error.transcription",
					"Bots couldn't hear that. Check your speech model and try again.",
				),
			})[error],
		[t],
	);
}

/** Starts a call, or joins the live one. */
export function CallButton({
	call,
}: Readonly<{
	call: Pick<ConversationCallState, "live" | "joined" | "start">;
}>) {
	const { t } = useTranslation("bots");
	const label = call.live
		? t("crew.call.join", "Join")
		: t("crew.call.start", "Start a call");
	return (
		<Button
			type="button"
			variant="ghost"
			size="icon"
			className="shrink-0 text-muted-foreground"
			disabled={call.joined}
			onClick={() => void call.start()}
			aria-label={label}
			title={label}
		>
			<PhoneIcon aria-hidden />
		</Button>
	);
}

function useElapsed(since: number | null): number {
	const [origin] = useState(() => since ?? Date.now());
	const start = since ?? origin;
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), 1000);
		return () => clearInterval(timer);
	}, []);
	return Math.max(0, now - start);
}

function CallPerson({
	sub,
	muted,
	speaking,
	listening,
	name,
}: Readonly<{
	sub: string;
	muted: boolean;
	speaking: boolean;
	listening: boolean;
	name: string;
}>) {
	const { t } = useTranslation("bots");
	const transcribed = listening
		? t("crew.call.transcribedPerson", "{{name}} lets bots listen", { name })
		: null;
	const state = transcribed
		? transcribed
		: speaking
			? t("crew.call.speaking", "{{name}} is speaking", { name })
			: muted
				? t("crew.call.mutedPerson", "{{name}} is muted", { name })
				: name;
	return (
		<li
			className={cn(
				"relative flex rounded-full ring-2 transition-shadow",
				speaking ? "ring-primary" : "ring-transparent",
			)}
			data-speaking={speaking}
			aria-label={state}
			title={state}
		>
			<IdentityAvatar subject={sub} size={30} />
			{listening && (
				<span
					data-slot="call-listening-badge"
					className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full bg-primary text-primary-foreground ring-2 ring-card"
				>
					<AudioLinesIcon className="size-2.5" aria-hidden />
				</span>
			)}
			{muted && (
				<span className="absolute -right-1 -bottom-1 flex size-4 items-center justify-center rounded-full bg-muted text-muted-foreground ring-2 ring-card">
					<MicOffIcon className="size-2.5" aria-hidden />
				</span>
			)}
		</li>
	);
}

const EDITABLE =
	'input, textarea, select, [contenteditable=""], [contenteditable="true"]';
const OPEN_OVERLAY =
	'[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"], [role="menu"][data-state="open"]';

export function shouldInterruptOnKey(
	event: Pick<KeyboardEvent, "key" | "defaultPrevented" | "target">,
	doc: Pick<Document, "querySelector"> = document,
): boolean {
	if (event.key !== "Escape" || event.defaultPrevented) return false;
	const target = event.target;
	if (
		target instanceof Element &&
		(target.closest(EDITABLE) || (target as HTMLElement).isContentEditable)
	)
		return false;
	return doc.querySelector(OPEN_OVERLAY) === null;
}

/** Escape stops the speaking bot while you are in the call, unless the key belongs to something else. */
function useInterruptShortcut(active: boolean, interrupt: () => void) {
	useEffect(() => {
		if (!active) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (!shouldInterruptOnKey(event)) return;
			event.preventDefault();
			interrupt();
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [active, interrupt]);
}

function InterruptBotButton({
	botName,
	onInterrupt,
}: Readonly<{ botName: string; onInterrupt: () => void }>) {
	const { t } = useTranslation("bots");
	const label = t("crew.call.interrupt", "Interrupt {{name}}", {
		name: botName,
	});
	return (
		<Button
			type="button"
			variant="secondary"
			size="sm"
			className="shrink-0"
			data-slot="call-interrupt"
			onClick={onInterrupt}
			aria-label={label}
			aria-keyshortcuts="Escape"
			title={t("crew.call.interruptHint", "Interrupt {{name}} (Esc)", {
				name: botName,
			})}
		>
			<HandIcon aria-hidden />
			<span className="@max-[820px]/bots:hidden">
				{t("crew.call.interruptShort", "Interrupt")}
			</span>
		</Button>
	);
}

/** Lets bots read your side of the call; off until you switch it on, and unavailable without a speech model. */
export function ListeningToggle({
	call,
}: Readonly<{
	call: Pick<ConversationCallState, "listening" | "canListen" | "setListening">;
}>) {
	const { t } = useTranslation("bots");
	const label = t("crew.call.listen", "Let bots listen");
	const hint = call.canListen
		? t(
				"crew.call.listenHint",
				"Your microphone is transcribed on your device or your speech model and posted as text. Bots can read it, and saying a bot's name wakes it.",
			)
		: t(
				"crew.call.listenNeedsModel",
				"Add a speech-to-text model to your profile to let bots listen.",
			);
	return (
		<div
			data-slot="call-listen-toggle"
			className="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground"
			title={hint}
		>
			<Switch
				checked={call.listening}
				disabled={!call.canListen}
				onCheckedChange={call.setListening}
				aria-label={label}
				aria-description={hint}
			/>
			<span className="@max-[820px]/bots:hidden">{label}</span>
		</div>
	);
}

/** Shows participants and controls, with shared screens above camera previews. */
export function ConversationCallBar({
	call,
	me,
	directory,
}: Readonly<{
	call: Pick<
		ConversationCallState,
		| "live"
		| "joined"
		| "join"
		| "leave"
		| "toggleMute"
		| "listening"
		| "transcriptDraft"
		| "canListen"
		| "botsPresent"
		| "setListening"
		| "speakingBot"
		| "interruptSpeech"
		| "cameraOn"
		| "screenSharing"
		| "canUseCamera"
		| "canShareScreen"
		| "mediaPending"
		| "mediaError"
		| "toggleCamera"
		| "toggleScreenShare"
		| "videos"
	>;
	me?: string | null;
	directory: CrewDirectory;
}>) {
	const { t } = useTranslation("bots");
	const elapsed = useElapsed(call.live?.startedAt ?? null);
	const speakingBot = call.joined ? (call.speakingBot ?? null) : null;
	useInterruptShortcut(
		speakingBot !== null && call.interruptSpeech !== undefined,
		call.interruptSpeech,
	);
	if (!call.live) return null;
	const mine = call.live.participants.find((p) => p.sub === me);
	const muteLabel = mine?.muted
		? t("crew.call.unmute", "Unmute microphone")
		: t("crew.call.mute", "Mute microphone");
	const cameraLabel = call.cameraOn
		? t("crew.call.cameraOff", "Turn camera off")
		: t("crew.call.cameraOn", "Turn camera on");
	const screenLabel = call.screenSharing
		? t("crew.call.screenOff", "Stop sharing screen")
		: t("crew.call.screenOn", "Share screen");
	const mediaError = call.mediaError
		? call.mediaError.source === "camera"
			? call.mediaError.reason === "denied"
				? t(
						"crew.call.cameraDenied",
						"Camera access was denied. Allow camera access in your browser or system settings.",
					)
				: t(
						"crew.call.cameraUnavailable",
						"The camera is unavailable. Check that it is connected and another app is not using it.",
					)
			: call.mediaError.reason === "denied"
				? t(
						"crew.call.screenDenied",
						"Screen sharing was cancelled or denied. Choose Share screen to try again.",
					)
				: t(
						"crew.call.screenUnavailable",
						"Screen sharing is unavailable here. Open this meeting in a browser that supports screen sharing.",
					)
		: null;
	return (
		<section
			data-slot="conversation-call-bar"
			aria-label={t("crew.call.live", "Call in progress")}
			className="shrink-0 border-b bg-muted/40"
		>
			<div className="flex flex-wrap items-center gap-2 px-4.5 py-2 @max-[820px]/bots:px-3">
				<ul className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden p-0.5">
					{call.live.participants.map((p) => (
						<CallPerson
							key={p.sub}
							sub={p.sub}
							muted={p.muted}
							speaking={p.speaking}
							listening={p.listening}
							name={directory(p.sub).name}
						/>
					))}
				</ul>
				<span
					className="shrink-0 text-xs tabular-nums text-muted-foreground"
					data-slot="call-duration"
				>
					{formatClock(elapsed)}
				</span>
				{call.joined ? (
					<>
						{speakingBot !== null && (
							<InterruptBotButton
								botName={directory(speakingBot).name}
								onInterrupt={call.interruptSpeech}
							/>
						)}
						{call.botsPresent && <ListeningToggle call={call} />}
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="shrink-0"
							aria-label={cameraLabel}
							aria-pressed={call.cameraOn ?? false}
							title={
								call.canUseCamera
									? cameraLabel
									: t(
											"crew.call.cameraUnsupported",
											"Camera sharing is unavailable on this device.",
										)
							}
							disabled={!call.canUseCamera || Boolean(call.mediaPending)}
							onClick={() => void call.toggleCamera()}
						>
							{call.cameraOn ? (
								<VideoIcon aria-hidden />
							) : (
								<VideoOffIcon aria-hidden />
							)}
						</Button>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="shrink-0"
							aria-label={screenLabel}
							aria-pressed={call.screenSharing ?? false}
							title={
								call.canShareScreen
									? screenLabel
									: t(
											"crew.call.screenUnsupported",
											"Screen sharing is unavailable here. Use a supported desktop browser.",
										)
							}
							disabled={!call.canShareScreen || Boolean(call.mediaPending)}
							onClick={() => void call.toggleScreenShare()}
						>
							{call.screenSharing ? (
								<MonitorOffIcon aria-hidden />
							) : (
								<MonitorUpIcon aria-hidden />
							)}
						</Button>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="shrink-0 text-muted-foreground"
							aria-pressed={mine?.muted ?? false}
							onClick={call.toggleMute}
							aria-label={muteLabel}
							title={muteLabel}
						>
							{mine?.muted ? (
								<MicOffIcon aria-hidden />
							) : (
								<MicIcon aria-hidden />
							)}
						</Button>
						<Button
							type="button"
							variant="destructive"
							size="icon"
							className="shrink-0"
							onClick={() => void call.leave()}
							aria-label={t("crew.call.leave", "Leave call")}
							title={t("crew.call.leave", "Leave call")}
						>
							<PhoneOffIcon aria-hidden />
						</Button>
					</>
				) : (
					<Button type="button" size="sm" onClick={() => void call.join()}>
						<PhoneIcon aria-hidden />
						{t("crew.call.join", "Join")}
					</Button>
				)}
			</div>
			{call.joined && call.listening && call.transcriptDraft && (
				<output
					data-slot="call-transcript-draft"
					aria-label={t("crew.call.liveTranscript", "Live transcript")}
					className="block max-h-24 overflow-y-auto px-4.5 pb-2 text-sm text-muted-foreground @max-[820px]/bots:px-3"
				>
					{call.transcriptDraft}
				</output>
			)}
			{mediaError && (
				<p role="alert" className="px-4 pb-2 text-sm text-destructive">
					{mediaError}
				</p>
			)}
			{call.joined && (call.videos?.length ?? 0) > 0 && (
				<div
					data-slot="call-video-grid"
					className="grid max-h-[55vh] grid-cols-2 gap-2 overflow-y-auto px-3 pb-3 sm:grid-cols-3"
				>
					{[...(call.videos ?? [])]
						.sort(
							(a, b) =>
								Number(b.kind === "screen") - Number(a.kind === "screen"),
						)
						.map((video) => (
							<CallVideoTile
								key={video.id}
								video={video}
								name={directory(video.sub).name}
							/>
						))}
				</div>
			)}
		</section>
	);
}

/** "Felix started a call" in the transcript, with Join while the call is live; "Call ended · 12 min" after. */
export function CallLine({
	call,
	directory,
	thread,
}: Readonly<{
	call: ConversationCall;
	directory: CrewDirectory;
	thread?: ThreadCall;
}>) {
	const { t } = useTranslation("bots");
	const duration = useCallDuration();
	const person = directory(call.senderId);
	const text =
		call.durationMs !== undefined
			? t("crew.call.ended", "Call ended · {{duration}}", {
					duration: duration(call.durationMs),
				})
			: person.isMe
				? t("crew.call.startedYou", "You started a call")
				: t("crew.call.started", "{{name}} started a call", {
						name: person.name,
					});
	const joinable =
		call.durationMs === undefined &&
		thread?.liveCallId === call.id &&
		!thread.joined;
	return (
		<div
			data-slot="call-line"
			className="flex items-center justify-center gap-2 px-3 text-xs leading-normal text-muted-foreground"
		>
			<PhoneIcon className="size-3.5 shrink-0" aria-hidden />
			<span className="truncate">{text}</span>
			{joinable && (
				<Button
					type="button"
					size="sm"
					variant="outline"
					className="h-6 px-2 text-xs"
					onClick={thread?.onJoin}
				>
					{t("crew.call.join", "Join")}
				</Button>
			)}
		</div>
	);
}
