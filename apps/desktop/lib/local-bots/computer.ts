import type {
	IBotComputerHost,
	IBotComputerUpdate,
	IBotComputerView,
	IBotInputEvent,
	IBotPairedScreenDevice,
	IBotScreenPairing,
	IBotScreenPairingCode,
} from "@flow-like/flow-like-ui";
import {
	RemoteBotComputerHost,
	type RemoteHostOptions,
} from "@flow-like/flow-like-ui/lib/bot-screens/remote-computer-host";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { detectBarcodes } from "../../components/composer-barcode-scanner";
import { isMobileDevice } from "../platform";

const ACTIVE_POLL_MS = 2000;
const ASLEEP_POLL_MS = 5000;
const CONTROL_POLL_MS = 600;

/**
 * The browser and screen of the bots that run on this desktop, and of device bots over the
 * encrypted device tunnel.
 */
export class BotComputerHost implements IBotComputerHost {
	view(botId: string, deviceId?: string | null): Promise<IBotComputerView> {
		return deviceId
			? invoke<IBotComputerView>("device_bot_view", { deviceId, botId })
			: invoke<IBotComputerView>("bot_computer_view", { botId });
	}

	stop(botId: string, deviceId?: string | null): Promise<void> {
		return deviceId
			? invoke<void>("device_bot_stop", { deviceId, botId })
			: invoke<void>("bot_computer_stop", { botId });
	}

	takeOver(botId: string, deviceId?: string | null): Promise<void> {
		return deviceId
			? invoke<void>("device_bot_take_over", { deviceId, botId })
			: invoke<void>("bot_computer_take_over", { botId });
	}

	handBack(botId: string, deviceId?: string | null): Promise<void> {
		return deviceId
			? invoke<void>("device_bot_hand_back", { deviceId, botId })
			: invoke<void>("bot_computer_hand_back", { botId });
	}

	input(
		botId: string,
		event: IBotInputEvent,
		deviceId?: string | null,
	): Promise<void> {
		if (!deviceId)
			return Promise.reject(
				new Error(
					"A bot on this computer is driven with your own mouse and keyboard",
				),
			);
		return invoke<void>("device_bot_input", { deviceId, botId, event });
	}
}

/** Pairs other devices with the screen host of this desktop through its native commands. */
export const desktopScreenPairing: IBotScreenPairing = {
	start: () => invoke<IBotScreenPairingCode>("bot_screens_pairing_start"),
	list: () => invoke<IBotPairedScreenDevice[]>("bot_screens_paired"),
	remove: (keyId) => invoke<void>("bot_screens_unpair", { keyId }),
	onPaired(listener) {
		let stopped = false;
		let unlisten: (() => void) | undefined;
		listen<{ key_id: string; label: string }>("bot-screens-paired", (event) =>
			listener(event.payload),
		)
			.then((off) => {
				if (stopped) off();
				else unlisten = off;
			})
			.catch(() => {});
		return () => {
			stopped = true;
			unlisten?.();
		};
	},
};

type Listener = (update: IBotComputerUpdate) => void;
type Source = "native" | "remote";

const pollDelay = (view: IBotComputerView) =>
	view.taken_over
		? CONTROL_POLL_MS
		: view.state === "active"
			? ACTIVE_POLL_MS
			: ASLEEP_POLL_MS;

const asUpdate = (view: IBotComputerView): IBotComputerUpdate => ({
	status: view.state === "active" ? "live" : "asleep",
	view,
});

/**
 * The desktop's computer view: bots on this computer and device bots through the native
 * commands, every other bot of the account through a live connection to the desktop app that runs
 * it. The picture comes from the native side while the bot is active here, else from the remote.
 */
export class CompositeComputerHost implements IBotComputerHost {
	readonly devices = true;
	readonly pairing = desktopScreenPairing;
	readonly scanBarcodes: IBotComputerHost["scanBarcodes"];
	private readonly shown = new Map<string, Source>();

	constructor(
		private readonly native: BotComputerHost,
		private readonly remote: RemoteBotComputerHost,
	) {
		this.scanBarcodes = remote.scanBarcodes;
	}

	view(botId: string, deviceId?: string | null): Promise<IBotComputerView> {
		return this.native.view(botId, deviceId);
	}

	watch(
		botId: string,
		deviceId: string | null | undefined,
		listener: Listener,
	): () => void {
		return deviceId
			? this.pollDevice(botId, deviceId, listener)
			: this.watchLocal(botId, listener);
	}

	retry(botId: string): void {
		this.remote.retry(botId);
	}

	pairScreen(botId: string, link: string): Promise<void> {
		return this.remote.pairScreen(botId, link);
	}

	forgetScreen(botId: string): Promise<void> {
		return this.remote.forgetScreen(botId);
	}

	private poll(
		read: () => Promise<IBotComputerView>,
		onView: (view: IBotComputerView) => void,
		onError: (error: unknown) => void,
	): () => void {
		let stopped = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const tick = async () => {
			let delay = ASLEEP_POLL_MS;
			try {
				const view = await read();
				if (stopped) return;
				delay = pollDelay(view);
				onView(view);
			} catch (error) {
				if (stopped) return;
				onError(error);
			}
			if (!stopped) timer = setTimeout(() => void tick(), delay);
		};
		void tick();
		return () => {
			stopped = true;
			clearTimeout(timer);
		};
	}

	private pollDevice(
		botId: string,
		deviceId: string,
		listener: Listener,
	): () => void {
		return this.poll(
			() => this.native.view(botId, deviceId),
			(view) => listener(asUpdate(view)),
			(error) =>
				listener({
					status: "unreachable",
					error: error instanceof Error ? error.message : String(error),
				}),
		);
	}

	private watchLocal(botId: string, listener: Listener): () => void {
		let nativeView: IBotComputerView | null = null;
		let nativeError: string | undefined;
		let remoteUpdate: IBotComputerUpdate | undefined;
		let stopRemote: (() => void) | null = null;
		let closed = false;

		const publish = () => {
			if (closed) return;
			if (nativeView?.state === "active") {
				this.shown.set(botId, "native");
				return listener(asUpdate(nativeView));
			}
			if (remoteUpdate?.status === "live") {
				this.shown.set(botId, "remote");
				return listener(remoteUpdate);
			}
			this.shown.set(botId, "native");
			if (remoteUpdate?.status === "unpaired") return listener(remoteUpdate);
			if (nativeView) return listener({ status: "asleep", view: nativeView });
			listener(
				nativeError
					? { status: "unreachable", error: nativeError }
					: { status: "connecting" },
			);
		};
		const syncRemote = () => {
			const needed = nativeView?.state !== "active";
			if (needed && !stopRemote) {
				stopRemote = this.remote.watch(botId, null, (update) => {
					remoteUpdate = update;
					publish();
				});
			} else if (!needed && stopRemote) {
				stopRemote();
				stopRemote = null;
				remoteUpdate = undefined;
			}
		};
		const stopPoll = this.poll(
			() => this.native.view(botId),
			(view) => {
				nativeView = view;
				nativeError = undefined;
				syncRemote();
				publish();
			},
			(error) => {
				nativeError = error instanceof Error ? error.message : String(error);
				syncRemote();
				publish();
			},
		);
		return () => {
			closed = true;
			stopPoll();
			stopRemote?.();
			this.shown.delete(botId);
		};
	}

	private source(botId: string, deviceId?: string | null): IBotComputerHost {
		return !deviceId && this.shown.get(botId) === "remote"
			? this.remote
			: this.native;
	}

	stop(botId: string, deviceId?: string | null): Promise<void> {
		return this.source(botId, deviceId).stop(botId, deviceId);
	}

	takeOver(botId: string, deviceId?: string | null): Promise<void> {
		return this.source(botId, deviceId).takeOver(botId, deviceId);
	}

	handBack(botId: string, deviceId?: string | null): Promise<void> {
		return this.source(botId, deviceId).handBack(botId, deviceId);
	}

	input(
		botId: string,
		event: IBotInputEvent,
		deviceId?: string | null,
	): Promise<void> {
		return this.source(botId, deviceId).input(botId, event, deviceId);
	}
}

/** The computer host of this app: composite on the desktop, remote only on a phone. */
export function createComputerHost(
	options: Omit<RemoteHostOptions, "excludeHost" | "scanBarcodes">,
): IBotComputerHost {
	const scanBarcodes = isTauri() ? detectBarcodes : undefined;
	if (isMobileDevice())
		return new RemoteBotComputerHost({ ...options, scanBarcodes });
	let hostId: Promise<string | null> | undefined;
	const excludeHost = () => {
		hostId ??= invoke<string>("bot_screens_host_id").catch(() => null);
		return hostId;
	};
	return new CompositeComputerHost(
		new BotComputerHost(),
		new RemoteBotComputerHost({ ...options, excludeHost, scanBarcodes }),
	);
}
