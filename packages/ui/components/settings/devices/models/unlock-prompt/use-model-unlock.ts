"use client";

import { useCallback, useEffect, useState } from "react";
import {
	type DeviceUnlockRequest,
	type ModelUnlockFailure,
	type ModelUnlockPrompt,
	type VaultLookup,
	deviceUnlockRequest,
	unlockFailure,
	withPrompts,
	withoutPrompt,
} from "./model-unlock";

/** The host side of the prompt: the desktop wires its connector commands and device storage. */
export interface ModelUnlockBridge {
	/** Prompts shown before this window listened. */
	pending(): Promise<ModelUnlockPrompt[]>;
	/** Resolves to the function that stops listening. */
	listen(
		onRequested: (prompt: ModelUnlockPrompt) => void,
		onClosed: (promptId: string) => void,
	): Promise<() => void>;
	vault(deviceId: string): Promise<VaultLookup>;
	unlock(request: DeviceUnlockRequest): Promise<void>;
	decline(promptId: string): Promise<void>;
}

/** Prompts in arrival order; the first one is shown. */
export function usePromptQueue(bridge: ModelUnlockBridge) {
	const [queue, setQueue] = useState<ModelUnlockPrompt[]>([]);
	const add = useCallback((prompts: ModelUnlockPrompt[]) => {
		setQueue((current) => withPrompts(current, prompts));
	}, []);
	const remove = useCallback((promptId: string) => {
		setQueue((current) => withoutPrompt(current, promptId));
	}, []);

	useEffect(() => {
		let stopped = false;
		let stop: (() => void) | undefined;
		const requested = (prompt: ModelUnlockPrompt) => add([prompt]);
		bridge
			.listen(requested, remove)
			.then((unlisten) => {
				if (stopped) unlisten();
				else stop = unlisten;
				return bridge.pending();
			})
			.then(add)
			.catch(() => undefined);
		return () => {
			stopped = true;
			stop?.();
		};
	}, [bridge, add, remove]);

	return { current: queue[0], remove };
}

/** `undefined` while the vault of the device is read. */
export function useVaultLookup(bridge: ModelUnlockBridge, deviceId: string) {
	const [lookup, setLookup] = useState<VaultLookup>();
	useEffect(() => {
		let current = true;
		bridge
			.vault(deviceId)
			.catch((): VaultLookup => ({ kind: "blocked", block: "no_vault" }))
			.then((next) => {
				if (current) setLookup(next);
			});
		return () => {
			current = false;
		};
	}, [bridge, deviceId]);
	return lookup;
}

/** Unlock and decline for one prompt; `done` removes it from the queue. */
export function usePromptAnswer(
	bridge: ModelUnlockBridge,
	prompt: ModelUnlockPrompt,
	lookup: VaultLookup | undefined,
	done: (promptId: string) => void,
) {
	const [busy, setBusy] = useState(false);
	const [failure, setFailure] = useState<ModelUnlockFailure>();

	const unlock = useCallback(
		async (password: string, keepUnlocked: boolean) => {
			if (lookup?.kind !== "ready") return;
			setBusy(true);
			setFailure(undefined);
			try {
				await bridge.unlock(
					deviceUnlockRequest(
						lookup.scope,
						lookup.vault,
						password,
						keepUnlocked,
					),
				);
				done(prompt.id);
			} catch (error) {
				setFailure(unlockFailure(error));
				setBusy(false);
			}
		},
		[bridge, lookup, prompt.id, done],
	);

	const decline = useCallback(() => {
		done(prompt.id);
		bridge.decline(prompt.id).catch(() => undefined);
	}, [bridge, prompt.id, done]);

	return { busy, failure, unlock, decline };
}
