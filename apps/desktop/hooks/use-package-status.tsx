"use client";

import { type Event, type UnlistenFn, listen } from "@tauri-apps/api/event";
import { useEffect, useSyncExternalStore } from "react";

export type CompileStatus =
	| "idle"
	| "downloading"
	| "compiling"
	| "ready"
	| "error"
	| "stale";

interface PackageStatusEvent {
	packageId: string;
	status: CompileStatus;
}

let statusMap = new Map<string, CompileStatus>();
const listeners = new Set<() => void>();

function notifyListeners() {
	for (const fn of listeners) fn();
}

function subscribe(cb: () => void) {
	listeners.add(cb);
	return () => {
		listeners.delete(cb);
	};
}

function getSnapshot() {
	return statusMap;
}

let unlistenPromise: Promise<UnlistenFn> | null = null;

function ensureListener() {
	if (unlistenPromise) return;
	unlistenPromise = listen(
		"package-status",
		(event: Event<PackageStatusEvent>) => {
			const { packageId, status } = event.payload;
			if ((statusMap.get(packageId) ?? "idle") === status) return;
			const next = new Map(statusMap);
			if (status === "idle") {
				next.delete(packageId);
			} else {
				next.set(packageId, status);
			}
			statusMap = next;
			notifyListeners();
		},
	);
}

export function usePackageStatusMap(): Map<string, CompileStatus> {
	useEffect(() => {
		ensureListener();
	}, []);

	return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function usePackageStatus(packageId: string): CompileStatus {
	const map = usePackageStatusMap();
	return map.get(packageId) ?? "idle";
}
