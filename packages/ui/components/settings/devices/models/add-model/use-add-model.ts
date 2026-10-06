"use client";

import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import {
	listableModels,
	searchAllBits,
} from "../../../../../lib/bit/model-listing";
import type {
	ModelInstalled,
	ModelSettings,
	Residency,
} from "../../../../../lib/device-management/models";
import { type IBit, IBitTypes } from "../../../../../lib/schema";
import { useBackend } from "../../../../../state/backend-state";
import type { IBitState } from "../../../../../state/backend-state/bit-state";
import { useDeviceWorkspace } from "../../workspace";
import { useModelsAction } from "../use-models-action";
import {
	type ModelChoice,
	type ModelSource,
	filesToFingerprint,
	isHostableBit,
	modelSpecOf,
} from "./model-options";
import {
	type Fetcher,
	fingerprintAll,
	hubCandidate,
	huggingFaceCandidate,
	userBitCandidate,
} from "./model-reads";

/*
 * The add-model wizard's state outside the device: the hub catalog and the
 * person's Bits a device can host, looking a candidate up, and the install
 * itself (fingerprint small files, then one `install` command).
 */

const HUB_TYPES = [
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.SystemOne,
	IBitTypes.Embedding,
];

async function hubModels(bits: IBitState) {
	const all = await searchAllBits.call(bits, { bit_types: HUB_TYPES });
	return listableModels(all).filter(isHostableBit);
}

async function userModels(bits: IBitState) {
	return (await bits.listCustomBits()).filter(isHostableBit);
}

/** Hub models and the person's own Bits a device can host, read once their source is open. */
export function useSourceLists(source: ModelSource) {
	const { bitState } = useBackend();
	const { scopeKey } = useDeviceWorkspace();
	const hub = useQuery({
		queryKey: ["devices", scopeKey, "models", "add", "hub"],
		queryFn: () => hubModels(bitState),
		enabled: source === "hub",
		staleTime: 5 * 60_000,
		retry: false,
		meta: { persist: false },
	});
	const bits = useQuery({
		queryKey: ["devices", scopeKey, "models", "add", "bits"],
		queryFn: () => userModels(bitState),
		enabled: source === "bits",
		staleTime: 60_000,
		retry: false,
		meta: { persist: false },
	});
	return { hub, bits };
}

export interface SourcePick {
	source: ModelSource;
	/** A hub model or one of the person's Bits. */
	bit?: IBit;
	/** "owner/repo" or a huggingface.co URL. */
	reference: string;
}

function lookup(pick: SourcePick, bits: IBitState, fetcher: Fetcher) {
	if (pick.source === "huggingface")
		return huggingFaceCandidate(pick.reference, fetcher);
	if (!pick.bit)
		return Promise.reject(new Error("Choose a model before you continue."));
	return pick.source === "hub"
		? hubCandidate(pick.bit, bits, fetcher)
		: userBitCandidate(pick.bit, fetcher);
}

const messageOf = (error: unknown) =>
	error instanceof Error ? error.message : String(error);

/** Looks the picked model up: its versions, files, digests and facts. */
export function useCandidateLookup(fetcher: Fetcher) {
	const { bitState } = useBackend();
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const find = async (pick: SourcePick) => {
		setBusy(true);
		setError(undefined);
		try {
			return await lookup(pick, bitState, fetcher);
		} catch (failure) {
			setError(messageOf(failure));
			return undefined;
		} finally {
			setBusy(false);
		}
	};
	return { busy, error, find, clear: () => setError(undefined) };
}

export type InstallPhase =
	| { kind: "idle" }
	| { kind: "fingerprinting"; done: number; total: number }
	| { kind: "sending" }
	| { kind: "done"; result: ModelInstalled }
	| { kind: "failed"; message: string };

export interface InstallInput {
	choice: ModelChoice;
	modelId: string;
	settings: ModelSettings;
	residency: Residency;
}

/**
 * Fingerprints the small files without a digest, then sends one `install`.
 * Closing the wizard unmounts it: that stops the downloads, and nothing is
 * sent.
 */
export function useInstall(deviceId: string, fetcher: Fetcher) {
	const actions = useModelsAction(deviceId);
	const [phase, setPhase] = useState<InstallPhase>({ kind: "idle" });
	const running = useRef<AbortController | null>(null);
	useEffect(() => () => running.current?.abort(), []);
	const install = async (input: InstallInput) => {
		running.current?.abort();
		const controller = new AbortController();
		running.current = controller;
		const { signal } = controller;
		const files = filesToFingerprint(input.choice);
		setPhase({ kind: "fingerprinting", done: 0, total: files.length });
		try {
			const digests = await fingerprintAll(files, fetcher, {
				signal,
				onFile: (done) =>
					setPhase({ kind: "fingerprinting", done, total: files.length }),
			});
			const model = modelSpecOf(input.choice, digests);
			signal.throwIfAborted();
			setPhase({ kind: "sending" });
			const outcome = await actions
				.install({
					modelId: input.modelId,
					model,
					settings: input.settings,
					residency: input.residency,
				})
				.run({ confirmed: true });
			setPhase(
				outcome.status === "done"
					? { kind: "done", result: outcome.result }
					: { kind: "idle" },
			);
		} catch (failure) {
			if (!signal.aborted)
				setPhase({ kind: "failed", message: messageOf(failure) });
		}
	};
	return { phase, install };
}
