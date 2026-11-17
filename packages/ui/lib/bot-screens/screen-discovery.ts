import type { ScreenSignal, ScreenSignalDraft } from "./screen-protocol";

export const DISCOVERY_WINDOW_MS = 1500;

export interface SignalBus {
	readonly viewerId: string;
	send(message: ScreenSignalDraft): Promise<void>;
	subscribe(listener: (signal: ScreenSignal) => void): () => void;
}

export interface Discovery {
	/** Trusted host peer id to whether that host has a live picture of the bot. */
	hosts: Map<string, boolean>;
	/** Key id of each trusted host in `hosts`. */
	keyIds: Map<string, string>;
	active: string | null;
	/** Hosts that answered with a key this device did not pin. */
	strangers: number;
	/** One of those hosts has a live picture of the bot. */
	strangerActive: boolean;
	/** The name a stranger gave, the active one's first. */
	strangerLabel?: string;
}

/**
 * Greets every desktop host and collects who answers for this bot. Stops at the first host with a
 * live picture; `excludeHost` (this app's own host) is ignored as a responder. Only hosts that
 * `trusted` accepts (pinned key and the peer id it was paired from) are offered to; the rest are
 * counted as strangers.
 */
export function discoverHosts(
	bus: SignalBus,
	botId: string,
	options: {
		excludeHost?: string | null;
		trusted?: (keyId: string | undefined, from: string) => boolean;
		windowMs?: number;
		signal?: AbortSignal;
	} = {},
): Promise<Discovery> {
	return new Promise<Discovery>((resolve, reject) => {
		const hosts = new Map<string, boolean>();
		const keyIds = new Map<string, string>();
		const strangers = new Set<string>();
		let active: string | null = null;
		let strangerActive = false;
		let strangerLabel: string | undefined;
		let off = () => {};
		const finish = () => {
			clearTimeout(timer);
			off();
			options.signal?.removeEventListener("abort", finish);
			resolve({
				hosts,
				keyIds,
				active,
				strangers: strangers.size,
				strangerActive,
				strangerLabel,
			});
		};
		const timer = setTimeout(finish, options.windowMs ?? DISCOVERY_WINDOW_MS);
		off = bus.subscribe((signal) => {
			if (
				signal.type !== "here" ||
				signal.to !== bus.viewerId ||
				signal.bot_id !== botId ||
				signal.from === options.excludeHost
			)
				return;
			if (options.trusted && !options.trusted(signal.key_id, signal.from)) {
				strangers.add(signal.from);
				if (signal.label && (signal.active || !strangerLabel))
					strangerLabel = signal.label;
				strangerActive ||= signal.active;
				return;
			}
			if (signal.key_id) keyIds.set(signal.from, signal.key_id);
			hosts.set(signal.from, signal.active || hosts.get(signal.from) === true);
			if (signal.active && active === null) {
				active = signal.from;
				finish();
			}
		});
		if (options.signal?.aborted) return finish();
		options.signal?.addEventListener("abort", finish);
		bus
			.send({ type: "hello", from: bus.viewerId, bot_id: botId })
			.catch((error) => {
				clearTimeout(timer);
				off();
				options.signal?.removeEventListener("abort", finish);
				reject(error);
			});
	});
}
