"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type HubError,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import {
	deviceKeys,
	queries,
	releaseConfigOf,
} from "../../../../lib/device-management/hub/queries";
import type { GateResult } from "../../../../lib/device-management/model/types";
import type {
	ReleaseConfig,
	VerifiedRelease,
} from "../../../../lib/device-management/package";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import { useDeviceWorkspace, useGate } from "../workspace";

export type ReleaseCheck =
	/** The hub publishes no release trust: agents can't be packaged. */
	| { state: "missing" }
	/** Not taken yet: the hub checks have not passed. */
	| { state: "waiting" }
	| { state: "checking" }
	| { state: "verified"; release: VerifiedRelease; verifiedAt: number }
	/** The manifest arrived and failed verification. `detail` is the verifier's own sentence. */
	| { state: "rejected"; detail: string }
	/** The release server gave no usable answer. */
	| { state: "unreachable"; error: HubError };

export type ReadinessCheck =
	| { state: "checking" }
	| { state: "failed"; error: HubError }
	| {
			state: "loaded";
			data: DeviceSetupReadiness;
			/** Epoch milliseconds of this answer. */
			checkedAt: number;
			/** A newer check is running or failed; `data` is the last answer. */
			refreshing: boolean;
			error?: HubError;
	  };

export interface SetupChecks {
	readiness: ReadinessCheck;
	release: ReleaseCheck;
	config: ReleaseConfig | undefined;
	/** `setup_device` with the readiness and release facts: hub, limits, signed releases. */
	gate: GateResult;
	/** Everything step 0 asks for holds right now; the only state that may create a setup. */
	pass: boolean;
	/** The verified release, only while the checks are current. */
	verified: VerifiedRelease | undefined;
	recheck(): Promise<void>;
}

const TRANSPORT = new Set(["network", "timeout", "server_error"]);

function readinessCheckOf(
	answer: DeviceSetupReadiness | undefined,
	failure: unknown,
	checkedAt: number,
	current: boolean,
): ReadinessCheck {
	const error = failure ? toHubError(failure) : undefined;
	if (answer)
		return {
			state: "loaded",
			data: answer,
			checkedAt,
			refreshing: !current,
			...(error ? { error } : {}),
		};
	return error ? { state: "failed", error } : { state: "checking" };
}

/** A manifest that did not arrive is "unreachable"; one that arrived and failed verification is "rejected". */
function failedRelease(failure: unknown): ReleaseCheck {
	const error = toHubError(failure);
	if (TRANSPORT.has(error.code)) return { state: "unreachable", error };
	const { cause } = error;
	return {
		state: "rejected",
		detail: cause instanceof Error ? cause.message : error.message,
	};
}

interface ReleaseFacts {
	/** The hub record is loaded, so "no release trust" is a fact and not a gap. */
	trustKnown: boolean;
	configured: boolean;
	hubReady: boolean;
	/** A fetch or a pending re-verification. */
	pending: boolean;
	failure: unknown;
	data: VerifiedRelease | undefined;
	verifiedAt: number;
}

function releaseCheckOf(facts: ReleaseFacts): ReleaseCheck {
	if (!facts.configured)
		return { state: facts.trustKnown ? "missing" : "waiting" };
	if (!facts.hubReady) return { state: "waiting" };
	if (facts.pending) return { state: "checking" };
	if (facts.failure) return failedRelease(facts.failure);
	return facts.data
		? { state: "verified", release: facts.data, verifiedAt: facts.verifiedAt }
		: { state: "checking" };
}

/**
 * Every readiness check made while the setup is open is followed by a fresh
 * verification of the release: a running check cancels a release fetch that
 * belongs to the previous answer, and the result of the new one counts only
 * when no newer check has started in between.
 */
function useReverify(
	readinessFetching: boolean,
	verifiable: boolean,
	releaseKey: readonly unknown[],
	refetchRelease: () => Promise<unknown>,
): boolean {
	const queryClient = useQueryClient();
	const [pending, setPending] = useState(false);
	const generation = useRef(0);

	useEffect(() => {
		if (!readinessFetching) return;
		generation.current += 1;
		setPending(true);
		void queryClient.cancelQueries({ queryKey: releaseKey });
	}, [readinessFetching, queryClient, releaseKey]);

	useEffect(() => {
		if (!pending || readinessFetching) return;
		const mine = generation.current;
		const settled = () => {
			if (generation.current === mine) setPending(false);
		};
		if (verifiable) void refetchRelease().then(settled, settled);
		else settled();
	}, [pending, readinessFetching, verifiable, refetchRelease]);

	return pending;
}

/**
 * Step 0 of the setup: the hub's readiness gates the release, so a stale
 * manifest never opens the way to an enrollment (the area keeps the manifest
 * for agent updates; the setup takes it only after the hub checks pass).
 */
export function useSetupChecks(): SetupChecks {
	const { hub } = useDeviceWorkspace();
	const readinessQuery = useQuery(queries.readiness(hub));
	const support = useQuery(queries.hub(hub));
	const config = releaseConfigOf(support.data);
	const configured = !!config;

	const answer = readinessQuery.data;
	const fetching = readinessQuery.isFetching;
	const current = !!answer && !fetching && !readinessQuery.isError;
	const hubReady = current && answer.ready;
	const verifiable = hubReady && configured;

	const releaseQuery = useQuery({
		...queries.release(hub, config),
		enabled: verifiable,
	});
	const releaseKey = useMemo(
		() => deviceKeys.release(hub.scopeKey),
		[hub.scopeKey],
	);
	const reverify = useReverify(
		fetching,
		verifiable,
		releaseKey,
		releaseQuery.refetch,
	);

	const readiness = useMemo(
		() =>
			readinessCheckOf(
				answer,
				readinessQuery.error,
				readinessQuery.dataUpdatedAt,
				current,
			),
		[answer, readinessQuery.error, readinessQuery.dataUpdatedAt, current],
	);

	const trustKnown = !!support.data;
	const pending = releaseQuery.isFetching || reverify;
	const failure = releaseQuery.isError ? releaseQuery.error : undefined;
	const { data, dataUpdatedAt: verifiedAt } = releaseQuery;
	const release = useMemo(
		() =>
			releaseCheckOf({
				trustKnown,
				configured,
				hubReady,
				pending,
				failure,
				data,
				verifiedAt,
			}),
		[trustKnown, configured, hubReady, pending, failure, data, verifiedAt],
	);

	const gate = useGate("setup_device", undefined, {
		extra: {
			...(current ? { readinessOk: answer.ready } : {}),
			...(trustKnown ? { releaseTrust: configured } : {}),
		},
	});

	const verified = release.state === "verified" ? release.release : undefined;
	const pass = hubReady && gate.ok && !!verified;

	const refetchReadiness = readinessQuery.refetch;
	const recheck = useCallback(async () => {
		await refetchReadiness();
	}, [refetchReadiness]);

	return useMemo(
		() => ({ readiness, release, config, gate, pass, verified, recheck }),
		[readiness, release, config, gate, pass, verified, recheck],
	);
}
