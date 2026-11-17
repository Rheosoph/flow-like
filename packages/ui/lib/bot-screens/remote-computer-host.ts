import type { BarcodeDetect } from "../../components/interfaces/chat-default/chatbox/barcode-frames";
import type {
	IBotComputerHost,
	IBotComputerUpdate,
	IBotComputerView,
	IBotInputEvent,
	IBotScreenAccess,
} from "../../state/backend-state/bot-state";
import type { PlaygroundControl } from "./playground-codec";
import { type SignalBus, discoverHosts } from "./screen-discovery";
import { pairWithHost, parsePairingLink } from "./screen-pairing";
import {
	type ScreenConnection,
	ScreenNotPairedError,
	connectScreen,
} from "./screen-rtc";
import type { ScreenSealing } from "./screen-sealing";
import { ScreenSignaling } from "./screen-signaling";
import { ScreenViewer } from "./screen-viewer";

const ASLEEP_VIEW: IBotComputerView = {
	state: "asleep",
	kind: null,
	caption: null,
	image: null,
	age_ms: null,
};

export interface SignalingHandle extends SignalBus {
	ready(): Promise<unknown>;
	readonly iceServers?: IBotScreenAccess["ice_servers"];
	readonly iceExpiresAt?: number;
	close(): void;
}

export interface RemoteHostDeps {
	signaling: (access: () => Promise<IBotScreenAccess>) => SignalingHandle;
	discover: typeof discoverHosts;
	connect: (
		bus: SignalingHandle,
		options: Parameters<typeof connectScreen>[1],
	) => Promise<ScreenConnection>;
}

export interface RemoteHostTiming {
	asleepRetryMs: number;
	unreachableRetryMs: number;
	idleCloseMs: number;
	reconnectMs: number;
}

export interface RemoteHostOptions {
	access: () => Promise<IBotScreenAccess>;
	sealing: ScreenSealing;
	/** Reads QR codes natively where the webview has no `BarcodeDetector`. */
	scanBarcodes?: BarcodeDetect;
	/** How this device calls itself on the computer it pairs with. */
	deviceLabel?: () => string;
	/** Peer id of this app's own desktop host, which must not answer for itself. */
	excludeHost?: () => Promise<string | null>;
	deps?: Partial<RemoteHostDeps>;
	timing?: Partial<RemoteHostTiming>;
}

const DEFAULT_TIMING: RemoteHostTiming = {
	asleepRetryMs: 5000,
	unreachableRetryMs: 10_000,
	idleCloseMs: 10_000,
	reconnectMs: 1000,
};

const DEFAULT_DEPS: RemoteHostDeps = {
	signaling: (access) => new ScreenSignaling({ access }),
	discover: discoverHosts,
	connect: (bus, options) => connectScreen(bus, options),
};

const defaultDeviceLabel = () =>
	`Browser on ${globalThis.navigator?.platform || "this device"}`;

const NOT_CONNECTED = "This bot's computer is not connected yet";

function pause(ms: number, signal: AbortSignal, nudge: Set<() => void>) {
	return new Promise<void>((resolve) => {
		const done = () => {
			clearTimeout(timer);
			nudge.delete(done);
			signal.removeEventListener("abort", done);
			resolve();
		};
		const timer = setTimeout(done, ms);
		nudge.add(done);
		signal.addEventListener("abort", done);
	});
}

class BotScreenSession {
	private readonly abort = new AbortController();
	private readonly nudges = new Set<() => void>();
	private readonly listeners = new Set<(update: IBotComputerUpdate) => void>();
	private signaling: SignalingHandle | null = null;
	private viewer: ScreenViewer | null = null;
	private update: IBotComputerUpdate = { status: "connecting" };
	private idle: ReturnType<typeof setTimeout> | null = null;
	private hostKeyId: string | null = null;
	private droppedLabel: string | undefined;

	constructor(
		private readonly botId: string,
		private readonly options: RemoteHostOptions,
		private readonly deps: RemoteHostDeps,
		private readonly timing: RemoteHostTiming,
		private readonly onIdle: () => void,
	) {
		void this.run();
		this.armIdle();
	}

	get snapshot(): IBotComputerUpdate {
		return this.update;
	}

	watch(listener: (update: IBotComputerUpdate) => void): () => void {
		this.listeners.add(listener);
		this.disarmIdle();
		listener(this.update);
		return () => {
			this.listeners.delete(listener);
			if (this.listeners.size === 0) this.armIdle();
		};
	}

	touch(): void {
		if (this.listeners.size === 0) this.armIdle();
	}

	retry(): void {
		if (this.update.status === "live") return;
		this.set({ status: "connecting" });
		for (const wake of [...this.nudges]) wake();
	}

	disconnect(): void {
		this.viewer?.close();
		this.retry();
	}

	get pinnedKeyId(): string | null {
		return this.hostKeyId;
	}

	send(control: PlaygroundControl): Promise<void> {
		return this.viewer
			? this.viewer.send(control)
			: Promise.reject(new Error(NOT_CONNECTED));
	}

	close(): void {
		this.abort.abort();
		this.disarmIdle();
		this.viewer?.close();
		this.signaling?.close();
		this.signaling = null;
		this.listeners.clear();
	}

	private armIdle(): void {
		this.disarmIdle();
		this.idle = setTimeout(this.onIdle, this.timing.idleCloseMs);
	}

	private disarmIdle(): void {
		if (this.idle) clearTimeout(this.idle);
		this.idle = null;
	}

	private set(update: IBotComputerUpdate): void {
		if (this.abort.signal.aborted) return;
		this.update = update;
		for (const listener of [...this.listeners]) listener(update);
	}

	private async run(): Promise<void> {
		const { signal } = this.abort;
		while (!signal.aborted) {
			let wait = this.timing.unreachableRetryMs;
			try {
				wait = await this.attempt(signal);
			} catch (error) {
				if (signal.aborted) return;
				this.signaling?.close();
				this.signaling = null;
				this.set({
					status: "unreachable",
					error: error instanceof Error ? error.message : String(error),
				});
			}
			if (!signal.aborted) await pause(wait, signal, this.nudges);
		}
	}

	private async attempt(signal: AbortSignal): Promise<number> {
		this.signaling ??= this.deps.signaling(this.options.access);
		const signaling = this.signaling;
		await signaling.ready();
		const excludeHost = await this.options.excludeHost?.().catch(() => null);
		const pinned = new Map(
			(await this.options.sealing.trusted()).map((host) => [host.key_id, host]),
		);
		const found = await this.deps.discover(signaling, this.botId, {
			excludeHost,
			trusted: (keyId, from) =>
				keyId !== undefined && pinned.get(keyId)?.host_peer_id === from,
			signal,
		});
		if (signal.aborted) return 0;
		const host = found.active
			? pinned.get(found.keyIds.get(found.active) ?? "")
			: undefined;
		if (found.active && host) {
			this.hostKeyId = host.key_id;
			let connection: ScreenConnection;
			try {
				connection = await this.deps.connect(signaling, {
					hostPeerId: found.active,
					hostKey: host,
					sealing: this.options.sealing,
					botId: this.botId,
					iceServers: signaling.iceServers as RTCIceServer[] | undefined,
					iceExpiresAt: signaling.iceExpiresAt,
					signal,
				});
			} catch (error) {
				if (!(error instanceof ScreenNotPairedError)) throw error;
				await this.options.sealing.unpin(host.key_id);
				this.hostKeyId = null;
				this.droppedLabel = host.label;
				this.set({ status: "unpaired", host_label: host.label });
				return this.timing.asleepRetryMs;
			}
			await this.serve(connection, signal);
			return this.timing.reconnectMs;
		}
		if (found.hosts.size > 0) {
			this.set({ status: "asleep", view: ASLEEP_VIEW });
			return this.timing.asleepRetryMs;
		}
		if (
			found.strangerActive ||
			(found.strangers > 0 && pinned.size === 0 && !excludeHost)
		) {
			this.set({
				status: "unpaired",
				host_label: this.droppedLabel ?? found.strangerLabel,
			});
			return this.timing.asleepRetryMs;
		}
		this.set({ status: "unreachable" });
		return this.timing.unreachableRetryMs;
	}

	private async serve(
		connection: ScreenConnection,
		signal: AbortSignal,
	): Promise<void> {
		const viewer = new ScreenViewer(connection.stream);
		this.viewer = viewer;
		const stop = () => viewer.close();
		signal.addEventListener("abort", stop);
		const off = viewer.subscribe((view) => this.set({ status: "live", view }));
		this.set({ status: "live", view: viewer.view });
		await Promise.race([viewer.ended, connection.closed]);
		off();
		signal.removeEventListener("abort", stop);
		viewer.close();
		connection.close();
		this.viewer = null;
		this.set({ status: "connecting" });
	}
}

/**
 * Watches the bots of the signed-in account that run in a desktop app, from any app: one live
 * session per bot, shared by every watcher and closed shortly after the last one leaves.
 */
export class RemoteBotComputerHost implements IBotComputerHost {
	readonly devices = false;
	readonly scanBarcodes: BarcodeDetect | undefined;
	private readonly sessions = new Map<string, BotScreenSession>();
	private readonly deps: RemoteHostDeps;
	private readonly timing: RemoteHostTiming;

	constructor(private readonly options: RemoteHostOptions) {
		this.deps = { ...DEFAULT_DEPS, ...options.deps };
		this.timing = { ...DEFAULT_TIMING, ...options.timing };
		this.scanBarcodes = options.scanBarcodes;
	}

	private session(botId: string): BotScreenSession {
		let session = this.sessions.get(botId);
		if (!session) {
			const created = new BotScreenSession(
				botId,
				this.options,
				this.deps,
				this.timing,
				() => {
					if (this.sessions.get(botId) === created) this.sessions.delete(botId);
					created.close();
				},
			);
			this.sessions.set(botId, created);
			session = created;
		}
		return session;
	}

	private static refuseDevice(deviceId?: string | null): void {
		if (deviceId)
			throw new Error("A device bot's screen opens through its device");
	}

	async view(
		botId: string,
		deviceId?: string | null,
	): Promise<IBotComputerView> {
		RemoteBotComputerHost.refuseDevice(deviceId);
		const session = this.session(botId);
		session.touch();
		const { status, view, error } = session.snapshot;
		if (status === "unreachable")
			throw new Error(error ?? "No desktop app answered for this bot");
		return view ?? ASLEEP_VIEW;
	}

	watch(
		botId: string,
		deviceId: string | null | undefined,
		listener: (update: IBotComputerUpdate) => void,
	): () => void {
		if (deviceId) {
			listener({
				status: "unreachable",
				error: "A device bot's screen opens through its device",
			});
			return () => {};
		}
		return this.session(botId).watch(listener);
	}

	retry(botId: string): void {
		this.sessions.get(botId)?.retry();
	}

	async pairScreen(_botId: string, link: string): Promise<void> {
		const parsed = parsePairingLink(link);
		const { sealing } = this.options;
		const signaling = this.deps.signaling(this.options.access);
		try {
			await signaling.ready();
			const host = await pairWithHost(signaling, parsed, await sealing.own(), {
				label: (this.options.deviceLabel ?? defaultDeviceLabel)(),
			});
			await sealing.pin(host);
		} finally {
			signaling.close();
		}
		for (const session of this.sessions.values()) session.retry();
	}

	async forgetScreen(botId: string): Promise<void> {
		const keyId = this.sessions.get(botId)?.pinnedKeyId;
		if (!keyId) return;
		await this.options.sealing.unpin(keyId);
		this.sessions.get(botId)?.disconnect();
	}

	private control(botId: string, control: PlaygroundControl): Promise<void> {
		const session = this.sessions.get(botId);
		return session
			? session.send(control)
			: Promise.reject(new Error(NOT_CONNECTED));
	}

	stop(botId: string): Promise<void> {
		return this.control(botId, { type: "stop" });
	}

	takeOver(botId: string): Promise<void> {
		return this.control(botId, { type: "take_over" });
	}

	handBack(botId: string): Promise<void> {
		return this.control(botId, { type: "hand_back" });
	}

	input(botId: string, event: IBotInputEvent): Promise<void> {
		return this.control(botId, { type: "input", event });
	}
}
