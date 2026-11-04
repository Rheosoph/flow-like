import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { CallVideoTile } from "../../packages/ui/components/bots-tab/call-video-tile";
import type { CallVideoView } from "../../packages/ui/hooks/use-conversation-call";
import { createVideoCapture } from "../../packages/ui/lib/realtime/call-video";
import {
	type CallPeerLike,
	type CallProviderLike,
	type RemoteStreams,
	createCallEngine,
} from "../../packages/ui/lib/realtime/conversation-call";

declare global {
	interface Window {
		SimplePeer: new (
			options: object,
		) => CallPeerLike & {
			on(event: string, listener: (...args: never[]) => void): void;
			signal(data: unknown): void;
			destroy(): void;
		};
		callMediaTest: { run(): Promise<object>; cleanup(): void };
	}
}

const failures: string[] = [];
const watchers = new Set<() => void>();
let remote: RemoteStreams = new Map();
let cameraId: string | undefined;
let screenId: string | undefined;
let mic: MediaStream;
let animation: ReturnType<typeof setInterval>;

function provider(peer: CallPeerLike, id: string): CallProviderLike {
	return {
		room: { webrtcConns: new Map([[id, { peer }]]) },
		on() {},
		off() {},
	};
}

const sender = new window.SimplePeer({
	initiator: true,
	trickle: false,
	config: { iceServers: [] },
});
const receiver = new window.SimplePeer({
	initiator: false,
	trickle: false,
	config: { iceServers: [] },
});
sender.on("signal", (signal) => receiver.signal(signal));
receiver.on("signal", (signal) => sender.signal(signal));
sender.on("error", (error) => failures.push(String(error)));
receiver.on("error", (error) => failures.push(String(error)));
const outgoing = createCallEngine(provider(sender, "receiver"));
const incoming = createCallEngine(provider(receiver, "sender"));
outgoing.setAllowedPeers(new Set(["receiver"]));
incoming.onRemote((streams) => {
	remote = streams;
	for (const watcher of watchers) watcher();
});

// The picker requires interactive OS UI. Exercise its resulting track with a canvas stream.
Object.defineProperty(navigator.mediaDevices, "getDisplayMedia", {
	configurable: true,
	value: async () => {
		const canvas = document.createElement("canvas");
		canvas.width = 960;
		canvas.height = 540;
		const context = canvas.getContext("2d");
		if (!context) throw new Error("Canvas unavailable");
		const draw = () => {
			context.fillStyle = "#182635";
			context.fillRect(0, 0, 960, 540);
			context.fillStyle = "#8be4d0";
			context.font = "48px sans-serif";
			context.fillText("Shared screen", 65, 110);
			context.fillStyle = "#dce5ed";
			context.font = "26px sans-serif";
			context.fillText(`WebRTC frame ${Date.now()}`, 65, 180);
		};
		draw();
		animation = setInterval(draw, 80);
		return canvas.captureStream(15);
	},
});

const capture = createVideoCapture((state) => {
	cameraId = state.camera?.id;
	screenId = state.screen?.id;
	outgoing.setLocalStreams(
		[mic, state.camera, state.screen].filter((stream): stream is MediaStream =>
			Boolean(stream),
		),
	);
	for (const watcher of watchers) watcher();
});

async function waitFor(check: () => boolean, description: string) {
	const deadline = Date.now() + 15_000;
	while (!check()) {
		if (failures.length) throw new Error(failures.join("\n"));
		if (Date.now() > deadline)
			throw new Error(
				`Timed out: ${description}; received ${[...remote.values()]
					.flat()
					.map(
						(s) =>
							`${s.id}:${s.getTracks().map((t) => `${t.kind}/${t.readyState}`)}`,
					)
					.join(", ")}`,
			);
		await new Promise((done) => setTimeout(done, 30));
	}
}

function App() {
	const [, update] = useState(0);
	useEffect(() => {
		const change = () => update((n) => n + 1);
		watchers.add(change);
		return () => {
			watchers.delete(change);
		};
	}, []);
	const videos: CallVideoView[] = [];
	for (const stream of remote.get("sender") ?? []) {
		const kind =
			stream.id === screenId
				? "screen"
				: stream.id === cameraId
					? "camera"
					: null;
		if (kind)
			videos.push({ id: stream.id, sub: "sender", kind, stream, local: false });
	}
	return (
		<main>
			<h1>Camera and screen sharing</h1>
			<p>Two WebRTC peers, with microphone, camera, and screen streams.</p>
			<div className="tiles">
				{videos.map((video) => (
					<CallVideoTile key={video.id} video={video} name="Participant" />
				))}
			</div>
		</main>
	);
}

const style = document.createElement("style");
style.textContent =
	"body{margin:0;background:#101720;color:#e8eef5;font:16px system-ui}main{padding:32px;max-width:1120px;margin:auto}.tiles{display:grid;grid-template-columns:1fr 1fr;gap:20px}figure{position:relative;margin:0;background:black;border-radius:14px;overflow:hidden}video{display:block;width:100%;aspect-ratio:16/9;object-fit:contain}figcaption{padding:12px}figure button{position:absolute;right:10px;top:10px;width:34px;height:34px;border:0;border-radius:5px}figure svg{width:18px;height:18px}";
document.head.append(style);
const root = document.getElementById("root");
if (!root) throw new Error("Missing preview root");
createRoot(root).render(<App />);

window.callMediaTest = {
	async run() {
		mic = await navigator.mediaDevices.getUserMedia({ audio: true });
		outgoing.setLocalStreams([mic]);
		await capture.toggle("camera");
		await capture.toggle("screen");
		if (capture.state.error)
			throw new Error(JSON.stringify(capture.state.error));
		await waitFor(
			() => (remote.get("sender")?.length ?? 0) === 3,
			"three simultaneous streams",
		);
		const originalScreen = screenId;
		for (let cycle = 0; cycle < 4; cycle++) {
			const oldCamera = cameraId;
			await capture.toggle("camera");
			await waitFor(
				() => !remote.get("sender")?.some((stream) => stream.id === oldCamera),
				"camera removal",
			);
			if (
				screenId !== originalScreen ||
				capture.state.screen?.getVideoTracks()[0].readyState !== "live"
			)
				throw new Error("Camera toggle stopped the screen");
			await capture.toggle("camera");
			await waitFor(
				() =>
					remote.get("sender")?.some((stream) => stream.id === cameraId) ??
					false,
				"camera restart",
			);
		}
		const connection = (sender as unknown as { _pc: RTCPeerConnection })._pc;
		outgoing.setAllowedPeers(new Set());
		await waitFor(
			() => connection.getSenders().every((sender) => sender.track === null),
			"revoked participant stops receiving all media",
		);
		outgoing.setAllowedPeers(new Set(["receiver"]));
		await waitFor(
			() =>
				connection.getSenders().filter((sender) => sender.track !== null)
					.length === 3,
			"readmitted participant receives the same active streams",
		);
		await waitFor(
			() =>
				[...document.querySelectorAll("video")].length === 2 &&
				[...document.querySelectorAll("video")].every(
					(node) => node.readyState >= 2 && node.videoWidth > 0 && !node.paused,
				),
			"camera and screen tile playback",
		);
		return {
			streams: remote.get("sender")?.length,
			cameraToggleCycles: 4,
			videos: [...document.querySelectorAll("video")].map((node) => ({
				width: node.videoWidth,
				height: node.videoHeight,
				muted: node.muted,
				label: node.getAttribute("aria-label"),
			})),
		};
	},
	cleanup() {
		capture.dispose();
		for (const track of mic?.getTracks() ?? []) track.stop();
		clearInterval(animation);
		outgoing.dispose();
		incoming.dispose();
		sender.destroy();
		receiver.destroy();
	},
};
