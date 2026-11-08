import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useMemo } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import { BotDirectCall } from "../../packages/ui/components/bots-tab/bot-direct-call";
import {
	CallButton,
	ConversationCallBar,
} from "../../packages/ui/components/bots-tab/conversation-call";
import {
	foldInboxEntries,
	groupConversationItems,
} from "../../packages/ui/components/bots-tab/fold-entries";
import { callMentions } from "../../packages/ui/components/bots-tab/mentions";
import { SpokenLines } from "../../packages/ui/components/bots-tab/spoken-lines";
import type { CrewDirectory } from "../../packages/ui/components/bots-tab/use-crew-directory";
import { useBotCallSpeech } from "../../packages/ui/hooks/use-bot-call-speech";
import { useConversationCall } from "../../packages/ui/hooks/use-conversation-call";
import { useConversationRoom } from "../../packages/ui/hooks/use-conversation-room";
import { useInboxEntries } from "../../packages/ui/hooks/use-inbox";
import { useSpeechModel } from "../../packages/ui/hooks/use-speech-model";
import { encodeWav } from "../../packages/ui/lib/realtime/call-listening";
import {
	BackendContext,
	type IBackendState,
} from "../../packages/ui/state/backend-state";
import type {
	ISpeakRequest,
	ITranscribeRequest,
} from "../../packages/ui/state/backend-state/audio-state";
import type { IBot } from "../../packages/ui/state/backend-state/bot-state";
import type {
	IConversation,
	IInboxEntry,
	ISendInboxEntryRequest,
} from "../../packages/ui/state/backend-state/inbox-state";

createI18n({ language: "en" });
const params = new URLSearchParams(location.search);
const kind = params.get("kind") === "direct" ? "direct" : "group";
const voice = params.get("voice") ?? "default";
const me = "local";
const bots = [
	{ id: "bot_scout", name: "Scout", kind: "bot" as const },
	...(kind === "group"
		? [{ id: "bot_scribe", name: "Scribe", kind: "bot" as const }]
		: []),
];
const conversation = {
	id: "call-fixture",
	kind,
	title: "Bot crew",
	local: true,
	encryption: "plain",
	created_by: me,
	created_at: new Date().toISOString(),
	last_activity_at: new Date().toISOString(),
	members: [
		{ kind: "user", principal_id: me },
		...bots.map((bot) => ({ kind: "bot", principal_id: bot.id })),
	],
	unread_count: 0,
	last_entry: null,
} as IConversation;
const bot = {
	id: bots[0].id,
	name: bots[0].name,
	owner_id: me,
	local: true,
	tts_model: voice === "pinned" ? "fixture-voice" : null,
} as IBot;
const directory: CrewDirectory = (id) => ({
	id,
	name: bots.find((bot) => bot.id === id)?.name ?? "You",
	isMe: id === me,
	isBot: id !== me,
	isPending: false,
	isResolved: true,
	ttsModel: voice === "pinned" ? "fixture-voice" : null,
});
const stats = {
	transcriptions: [] as Array<
		Omit<ITranscribeRequest, "audio_base64"> & { bytes: number; wav: boolean }
	>,
	speech: [] as ISpeakRequest[],
	sent: [] as ISendInboxEntryRequest[],
	playbacks: 0,
	localAudioConnections: 0,
	peakOutput: 0,
	contexts: [] as AudioContext[],
	microphones: [] as MediaStream[],
};
let phrase = "";
let microphoneContext: AudioContext | null = null;
let microphoneGain: GainNode | null = null;
const NativeAudioContext = window.AudioContext;

// Observe native audio without replacing decoding, mixing, or playback.
class ObservedAudioContext extends NativeAudioContext {
	constructor(options?: AudioContextOptions) {
		super(options);
		stats.contexts.push(this);
	}
	createMediaStreamDestination() {
		const destination = super.createMediaStreamDestination();
		const analyser = this.createAnalyser();
		this.createMediaStreamSource(destination.stream).connect(analyser);
		const samples = new Float32Array(analyser.fftSize);
		const timer = setInterval(() => {
			if (this.state === "closed") {
				clearInterval(timer);
				return;
			}
			if (microphoneGain?.gain.value !== 0) return;
			analyser.getFloatTimeDomainData(samples);
			stats.peakOutput = Math.max(stats.peakOutput, ...samples.map(Math.abs));
		}, 20);
		return destination;
	}
	createBufferSource() {
		const source = super.createBufferSource();
		const start = source.start.bind(source);
		const connect = source.connect.bind(source);
		source.start = (...args) => {
			stats.playbacks++;
			stats.peakOutput = 0;
			start(...args);
		};
		source.connect = ((target: AudioNode, ...args: number[]) => {
			if (target === this.destination) stats.localAudioConnections++;
			return connect(target, ...args);
		}) as typeof source.connect;
		return source;
	}
}
window.AudioContext = ObservedAudioContext;

// Only the device boundary is synthetic. The production AudioWorklet captures each utterance.
Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
	configurable: true,
	value: async () => {
		microphoneContext = new NativeAudioContext();
		await microphoneContext.resume();
		const destination = microphoneContext.createMediaStreamDestination();
		const oscillator = microphoneContext.createOscillator();
		oscillator.frequency.value = 440;
		microphoneGain = microphoneContext.createGain();
		microphoneGain.gain.value = 0;
		oscillator.connect(microphoneGain);
		microphoneGain.connect(destination);
		oscillator.start();
		stats.microphones.push(destination.stream);
		return destination.stream;
	},
});
let sequence = 0;
const entries: IInboxEntry[] = [];
function entry(sender: string, request: ISendInboxEntryRequest): IInboxEntry {
	const id = `entry-${++sequence}`;
	return {
		id,
		message_id: request.message_id,
		conversation_id: conversation.id,
		sender_id: sender,
		kind: request.kind,
		envelope: request.envelope,
		cursor: `${String(sequence).padStart(8, "0")}:${id}`,
		created_at: new Date().toISOString(),
	};
}
const queryClient = new QueryClient({
	defaultOptions: {
		queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
	},
});
const voiceBit = {
	id: "fixture-voice",
	type: "Tts",
	parameters: {
		provider: { provider_name: "hosted:openai" },
		default_voice: "alloy",
	},
};
const sttBit = {
	id: "fixture-stt",
	type: "Stt",
	parameters: { provider: { provider_name: "hosted:openai" } },
};
const backend = {
	capabilities: () => ({ canExecuteLocally: false }),
	bitState: {
		getBit: async () => voiceBit,
		getProfileBits: async () => [
			sttBit,
			...(voice === "none" ? [] : [voiceBit]),
		],
	},
	userState: {
		lookupUser: async () => null,
		lookupUsers: async () => [],
		getProfile: async () => null,
		getSettingsProfile: async () => ({
			hub_profile: { id: "fixture-profile" },
		}),
	},
	inboxState: {
		createConversation: async () => conversation,
		getConversation: async () => conversation,
		listEntries: async () => ({
			entries: [...entries].reverse(),
			next_before: null,
		}),
		sendEntry: async (_id: string, request: ISendInboxEntryRequest) => {
			stats.sent.push(request);
			const sent = entry(me, request);
			entries.push(sent);
			// The model/inbox service boundary returns a final answer per addressed bot.
			if (request.kind === "transcript" && request.envelope.enc === "none") {
				const body = request.envelope.body as {
					text: string;
					mentions: string[];
				};
				for (const botId of body.mentions) {
					if (!bots.some((bot) => bot.id === botId))
						throw new Error(`Unknown bot ${botId}`);
					entries.push(
						entry(botId, {
							message_id: `reply-${sequence + 1}`,
							kind: "message",
							envelope: {
								v: 1,
								enc: "none",
								body: {
									text: `${directory(botId).name} heard: ${body.text}`,
									reply_to: sent.message_id,
								},
							},
						}),
					);
				}
			}
			return sent;
		},
	},
	audioState: {
		transcribe: async ({ audio_base64, ...request }: ITranscribeRequest) => {
			const bytes = atob(audio_base64);
			stats.transcriptions.push({
				...request,
				bytes: bytes.length,
				wav: bytes.startsWith("RIFF") && bytes.slice(8, 12) === "WAVE",
			});
			return { text: phrase };
		},
		speak: async (request: ISpeakRequest) => {
			stats.speech.push(request);
			if (voice === "failure")
				throw new Error("Fixture voice service unavailable");
			const samples = Float32Array.from(
				{ length: 22_400 },
				(_, index) => 0.25 * Math.sin((2 * Math.PI * 880 * index) / 16_000),
			);
			return new Blob([encodeWav(samples)], { type: "audio/wav" });
		},
	},
} as unknown as IBackendState;

function useHistory() {
	const history = useInboxEntries(conversation.id);
	const entries = useMemo(
		() => history.data?.pages.flatMap((page) => page.entries) ?? [],
		[history.data],
	);
	const items = useMemo(() => foldInboxEntries(entries, me), [entries]);
	return { entries, items, historyReady: history.data !== undefined };
}
function History() {
	const { items } = useHistory();
	return (
		<section aria-label="Conversation history">
			{groupConversationItems(items).map((block) =>
				block.type === "spoken" ? (
					<SpokenLines key={block.key} block={block} directory={directory} />
				) : null,
			)}
			{items.map((item) =>
				item.type === "message" && item.content.kind === "text" ? (
					<p key={item.id} data-testid="bot-reply" data-bot={item.senderId}>
						{item.content.text}
					</p>
				) : null,
			)}
		</section>
	);
}
function GroupCall() {
	const history = useHistory();
	const room = useConversationRoom(conversation, me);
	const stt = useSpeechModel();
	const call = useConversationCall(conversation, me, room, undefined, {
		stt,
		hasBots: true,
		mentionsIn: (text) => callMentions(text, bots, kind),
	});
	useBotCallSpeech({ conversation, ...history, directory, call });
	return (
		<>
			<CallButton call={call} />
			<ConversationCallBar call={call} directory={directory} me={me} />
			<History />
		</>
	);
}
const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
window.botCallTest = {
	async utterance(text: string) {
		phrase = text;
		if (!microphoneGain) throw new Error("Join before speaking");
		microphoneGain.gain.value = 0.2;
		await pause(1000);
		microphoneGain.gain.value = 0;
		await pause(1000);
	},
	snapshot: () => ({
		...stats,
		contexts: stats.contexts.map((context) => context.state),
		microphones: stats.microphones.map((stream) =>
			stream.getTracks().map((track) => track.readyState),
		),
		entries,
	}),
	async dispose() {
		await microphoneContext?.close();
	},
};
declare global {
	interface Window {
		botCallTest: {
			utterance(text: string): Promise<void>;
			snapshot(): object;
			dispose(): Promise<void>;
		};
	}
}
const style = document.createElement("style");
style.textContent =
	"body{background:#101720;color:#e8eef5;font:16px system-ui}main{max-width:900px;margin:40px auto}button{padding:10px;margin:8px;border-radius:8px}svg{width:20px;height:20px}section{padding:20px;border:1px solid #425366;margin:20px 0}ul{display:flex;gap:12px;list-style:none}[role=switch]{background:#596778}[role=switch][aria-checked=true]{background:#39a987}[data-slot=spoken-lines]{display:flex;gap:10px;color:#bac7d5}";
document.head.append(style);
const container = document.getElementById("root");
if (!container) throw new Error("Missing fixture root");
createRoot(container).render(
	<BackendContext.Provider value={backend}>
		<QueryClientProvider client={queryClient}>
			<main>
				<h1>{kind === "direct" ? "Call Scout" : "Call the bot crew"}</h1>
				{kind === "direct" ? (
					<BotDirectCall bot={bot}>
						{({ button, bar }) => (
							<>
								{button}
								{bar}
								<History />
							</>
						)}
					</BotDirectCall>
				) : (
					<GroupCall />
				)}
			</main>
			<Toaster />
		</QueryClientProvider>
	</BackendContext.Provider>,
);
