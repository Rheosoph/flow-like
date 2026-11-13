import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import { useEffect } from "react";
import { createRoot } from "react-dom/client";
import { AuthProvider } from "react-oidc-context";
import {
	CallButton,
	ConversationCallBar,
} from "../../packages/ui/components/bots-tab/conversation-call";
import type { CrewDirectory } from "../../packages/ui/components/bots-tab/use-crew-directory";
import { IncomingCallProvider } from "../../packages/ui/components/incoming-call-provider";
import { useConversationCall } from "../../packages/ui/hooks/use-conversation-call";
import {
	type ConversationRoom,
	useConversationRoom,
} from "../../packages/ui/hooks/use-conversation-room";
import {
	BackendContext,
	type IBackendState,
} from "../../packages/ui/state/backend-state";
import type {
	IConversation,
	ISendInboxEntryRequest,
} from "../../packages/ui/state/backend-state/inbox-state";

createI18n({ language: "en" });
const params = new URLSearchParams(location.search);
const me = params.get("me") ?? "alice";
const api = params.get("api") ?? "http://127.0.0.1:4402";
const people = ["alice", "bob", "carol"];
const bot = "bot_scout";

function conversationOf(id: string): IConversation {
	const group = id === "team-room";
	return {
		id,
		kind: group ? "group" : "direct",
		title: group ? "Team room" : null,
		encryption: "plain",
		created_by: "alice",
		created_at: "2026-11-11T08:00:00.000Z",
		last_activity_at: "2026-11-11T08:00:00.000Z",
		members: [
			...(group ? people : [me]).map((principal_id) => ({
				kind: "user",
				principal_id,
			})),
			{ kind: "bot", principal_id: bot },
		],
		unread_count: 0,
		last_entry: null,
	} as unknown as IConversation;
}

// Sockets are counted below the credential proxy, which is installed on first use.
const NativeWebSocket = window.WebSocket;
const sockets = {
	opened: 0,
	closes: [] as Array<{ code: number; path: string }>,
};
window.WebSocket = class extends NativeWebSocket {
	constructor(url: string | URL, protocols?: string | string[]) {
		super(url, protocols);
		const path = new URL(String(url)).pathname;
		this.addEventListener("open", () => {
			sockets.opened++;
		});
		this.addEventListener("close", (event) => {
			sockets.closes.push({ code: event.code, path });
		});
	}
};

const refuseCallModels = (url: string | URL) => {
	if (String(url).includes("call-turn.worker"))
		throw new Error("Call models are off in this fixture");
};
const NativeWorker = window.Worker;
window.Worker = class extends NativeWorker {
	constructor(url: string | URL, options?: WorkerOptions) {
		refuseCallModels(url);
		super(url, options);
	}
};
const NativeSharedWorker = window.SharedWorker;
if (NativeSharedWorker) {
	window.SharedWorker = class extends NativeSharedWorker {
		constructor(url: string | URL, options?: string | WorkerOptions) {
			refuseCallModels(url);
			super(url, options);
		}
	};
}

// Only the device boundary is synthetic: a steady tone stands in for a voice.
const microphones: MediaStream[] = [];
Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
	configurable: true,
	value: async () => {
		const context = new AudioContext();
		await context.resume();
		const destination = context.createMediaStreamDestination();
		const oscillator = context.createOscillator();
		oscillator.frequency.value = 440;
		const gain = context.createGain();
		gain.gain.value = 0.2;
		oscillator.connect(gain);
		gain.connect(destination);
		oscillator.start();
		microphones.push(destination.stream);
		return destination.stream;
	},
});

async function request<T>(path: string, body?: unknown): Promise<T> {
	const response = await fetch(`${api}${path}`, {
		method: body === undefined ? "GET" : "POST",
		headers: body === undefined ? {} : { "content-type": "application/json" },
		body: body === undefined ? undefined : JSON.stringify(body),
	});
	if (!response.ok)
		throw new Error(
			`${path} answered ${response.status}: ${await response.text()}`,
		);
	return response.json() as Promise<T>;
}

const accessRequests: string[] = [];
const backend = {
	capabilities: () => ({ canExecuteLocally: false }),
	userState: {
		getProfile: async () => ({ id: "fixture-profile", hub: api }),
		lookupUser: async () => null,
		lookupUsers: async () => [],
		listNotifications: async () =>
			request(`/fixture/notifications?sub=${encodeURIComponent(me)}`),
	},
	inboxState: {
		getConversation: async (id: string) => conversationOf(id),
		listEntries: async (id: string) =>
			request(`/fixture/entries?conversation=${encodeURIComponent(id)}`),
		listBlocks: async () => [],
		sendEntry: async (id: string, entry: ISendInboxEntryRequest) =>
			request("/fixture/entries", { conversation: id, sender: me, entry }),
		getConversationRealtimeAccess: async (id: string) => {
			accessRequests.push(id);
			return request("/fixture/realtime", { conversation: id, sub: me });
		},
	},
} as unknown as IBackendState;

const directory: CrewDirectory = (id) =>
	({
		id,
		name: id,
		isMe: id === me,
		isBot: id === bot,
		isPending: false,
		isResolved: true,
	}) as ReturnType<CrewDirectory>;

const rooms = new Map<string, ConversationRoom | undefined>();
const calls = new Map<string, { joined: boolean; error: string | null }>();

function Dock({ conversationId }: Readonly<{ conversationId: string }>) {
	const conversation = conversationOf(conversationId);
	const room = useConversationRoom(conversation, me);
	const call = useConversationCall(conversation, me, room);
	useEffect(() => {
		rooms.set(conversationId, room);
		return () => {
			rooms.delete(conversationId);
		};
	}, [conversationId, room]);
	useEffect(() => {
		calls.set(conversationId, { joined: call.joined, error: call.error });
		return () => {
			calls.delete(conversationId);
		};
	}, [conversationId, call.joined, call.error]);
	return (
		<section data-testid="dock" data-conversation={conversationId}>
			<h2>{conversation.title ?? "Scout"}</h2>
			{call.available && <CallButton call={call} />}
			<ConversationCallBar call={call} me={me} directory={directory} />
		</section>
	);
}

function Shell() {
	const selected = useSearchParams().get("conversation");
	return (
		<main>
			<h1>{me}</h1>
			{selected ? (
				<Dock key={selected} conversationId={selected} />
			) : (
				<p data-testid="no-dock">No conversation open</p>
			)}
			<IncomingCallProvider scope="room-call-fixture" userId={me} />
		</main>
	);
}

function connectedPeers(room: ConversationRoom | undefined): string[] {
	return [...(room?.provider.room?.webrtcConns ?? [])]
		.filter(([, conn]) => (conn as { connected?: boolean }).connected)
		.map(([id]) => id);
}

window.roomCallTest = {
	snapshot: () => ({
		me,
		sockets: { opened: sockets.opened, closes: [...sockets.closes] },
		accessRequests: [...accessRequests],
		rooms: Object.fromEntries(
			[...rooms].map(([id, room]) => [
				id,
				{
					up: room !== undefined,
					peers: connectedPeers(room).length,
					subs: [...(room?.awareness.getStates().values() ?? [])]
						.map((state) => (state as { sub?: unknown }).sub)
						.filter((sub): sub is string => typeof sub === "string")
						.sort(),
				},
			]),
		),
		calls: Object.fromEntries(calls),
		microphones: microphones.map((stream) =>
			stream.getTracks().map((track) => track.readyState),
		),
	}),
};
declare global {
	interface Window {
		roomCallTest: { snapshot(): object };
	}
}

const style = document.createElement("style");
style.textContent =
	"body{background:#101720;color:#e8eef5;font:16px system-ui}main{max-width:900px;margin:40px auto}button{padding:10px;margin:8px;border-radius:8px}svg{width:20px;height:20px}section{padding:20px;border:1px solid #425366;margin:20px 0}ul{display:flex;gap:12px;list-style:none}li[data-speaking=true]{outline:2px solid #39a987}[role=alertdialog]{position:fixed;inset:auto 20px 20px auto;padding:20px;background:#1c2733;border:1px solid #425366}";
document.head.append(style);
const container = document.getElementById("root");
if (!container) throw new Error("Missing fixture root");
const queryClient = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
createRoot(container).render(
	<AuthProvider
		authority="http://127.0.0.1:9/"
		client_id="room-call-fixture"
		redirect_uri={location.origin}
	>
		<BackendContext.Provider value={backend}>
			<QueryClientProvider client={queryClient}>
				<Shell />
			</QueryClientProvider>
		</BackendContext.Provider>
	</AuthProvider>,
);
