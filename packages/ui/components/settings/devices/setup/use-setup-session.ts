"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	PendingSetup,
	SetupRoute,
} from "../../../../lib/device-management/model/types";
import { useDevicesRoute } from "../routing/use-devices-route";
import { useDeviceWorkspace, usePendingSetups } from "../workspace";
import {
	NEW_SLOT,
	START_STEP,
	type SetupDraft,
	findDraft,
	freshDraft,
	isClosed,
	readDrafts,
	writeDraft,
} from "./setup-state";
import { useSetupLimits } from "./use-setup-facts";

interface Session {
	scopeKey: string;
	/** `new`, or the enrollment id in the URL. */
	slot: string;
	/** Where the draft is stored: a setup made here stays in `new` while it is open. */
	storedSlot: string;
	draft: SetupDraft | undefined;
	/** A new value remounts the flow: another setup starts with nothing held over. */
	epoch: number;
}

/** A setup of this window that still waits for its device moves under its own id, where Start instructions finds it. */
function archive(scopeKey: string, draft: SetupDraft) {
	const id = draft.created?.enrollmentId;
	if (id && !isClosed(draft)) writeDraft(scopeKey, id, draft);
}

/** `?flow=setup` without a step is a new setup; with one, this window's setup continues. */
function openSession(
	scopeKey: string,
	slot: string,
	newEntry: boolean,
	epoch: number,
): Session {
	const found = findDraft(readDrafts(scopeKey), slot);
	if (slot !== NEW_SLOT)
		return {
			scopeKey,
			slot,
			storedSlot: found?.slot ?? slot,
			draft: found?.draft,
			epoch,
		};
	const own = found?.draft;
	if (own && newEntry && (own.created || isClosed(own))) {
		archive(scopeKey, own);
		writeDraft(scopeKey, NEW_SLOT, undefined);
		return { scopeKey, slot, storedSlot: slot, draft: freshDraft(), epoch };
	}
	return {
		scopeKey,
		slot,
		storedSlot: slot,
		draft: own ?? freshDraft(),
		epoch,
	};
}

/** A pending setup this window has no draft for: only the steps after Save apply. */
function draftFromRecord(record: PendingSetup, lifetimeS: number): SetupDraft {
	return freshDraft({
		name: record.name,
		step: START_STEP,
		resumed: true,
		keySaved: true,
		created: {
			enrollmentId: record.enrollmentId,
			deviceId: record.deviceId ?? "",
			createdAt: record.createdAt ?? record.expiresAt - lifetimeS,
			expiresAt: record.expiresAt,
		},
	});
}

export interface SetupSession {
	scopeKey: string;
	slot: string;
	/** Changes when another setup opens: the flow's React key. */
	key: string;
	/** Undefined while a resumed setup is looked up in the pending setups. */
	draft: SetupDraft | undefined;
	update(patch: Partial<SetupDraft>): void;
	/** A fresh setup, optionally named like an expired one. */
	startNew(name?: string): void;
}

/** A resumed setup without a draft of this window: built from the pending setup the hub (or this computer) lists. */
function useRecordDraft(slot: string, known: boolean): SetupDraft | undefined {
	const pendingSetups = usePendingSetups();
	const { lifetimeS } = useSetupLimits();
	const wanted = known || slot === NEW_SLOT ? undefined : slot;
	const record = pendingSetups.find((setup) => setup.enrollmentId === wanted);
	return useMemo(
		() => (record ? draftFromRecord(record, lifetimeS) : undefined),
		[record, lifetimeS],
	);
}

/** The wizard's draft for the setup the URL names, kept in `sessionStorage` per account scope. */
export function useSetupSession(route: SetupRoute): SetupSession {
	const { scopeKey } = useDeviceWorkspace();
	const { navigate } = useDevicesRoute();
	const slot = route.enrollmentId ?? NEW_SLOT;
	const newEntry = route.step === undefined;
	const [session, setSession] = useState<Session>(() =>
		openSession(scopeKey, slot, newEntry, 0),
	);
	if (session.scopeKey !== scopeKey || session.slot !== slot)
		setSession(openSession(scopeKey, slot, newEntry, session.epoch + 1));

	const recordDraft = useRecordDraft(slot, !!session.draft);
	const draft = session.draft ?? recordDraft;
	const latestDraft = useRef(draft);
	latestDraft.current = draft;

	useEffect(() => {
		if (session.draft)
			writeDraft(session.scopeKey, session.storedSlot, session.draft);
	}, [session]);

	const update = useCallback((patch: Partial<SetupDraft>) => {
		setSession((current) => {
			const base = current.draft ?? latestDraft.current;
			return base ? { ...current, draft: { ...base, ...patch } } : current;
		});
	}, []);

	const startNew = useCallback(
		(name = "") => {
			const own = findDraft(readDrafts(scopeKey), NEW_SLOT)?.draft;
			if (own) archive(scopeKey, own);
			const fresh = freshDraft({ name });
			writeDraft(scopeKey, NEW_SLOT, fresh);
			if (slot === NEW_SLOT)
				setSession((current) => ({
					scopeKey,
					slot,
					storedSlot: NEW_SLOT,
					draft: fresh,
					epoch: current.epoch + 1,
				}));
			navigate({ screen: "setup" }, { replace: slot === NEW_SLOT });
		},
		[scopeKey, slot, navigate],
	);

	return {
		scopeKey,
		slot,
		key: `${session.scopeKey}|${session.slot}|${session.epoch}`,
		draft,
		update,
		startNew,
	};
}
