"use client";

import type { IHomeLayout } from "@flow-like/flow-like-ui/components/home/types";
import { useAssistantSurface } from "@flow-like/flow-like-ui/state/assistant-surface";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
	nativeSnapshotOwner,
	matchesNativeSnapshotOwner,
	type NativeSnapshotOwner,
} from "../lib/native-snapshot-owner";

interface NativeHomeProposal {
	owner: NativeSnapshotOwner;
	profile_id: string;
	layout: IHomeLayout;
	expected_fingerprint: string;
	candidate_fingerprint: string;
}

let synchronization: Promise<void> = Promise.resolve();
let displayed: NativeHomeProposal | undefined;

/** Capture the visible draft before a native run starts or continues. */
export function syncNativeHomeSnapshot(): Promise<void> {
	const expectedOwner = nativeSnapshotOwner();
	const snapshot = structuredClone(
		useAssistantSurface.getState().homeSurface?.getSnapshot(),
	);
	const next = synchronization
		.catch(() => {})
		.then(async () => {
			if (snapshot && !matchesNativeSnapshotOwner(expectedOwner))
				throw new Error(
					"The Home profile changed before its draft was captured.",
				);
			await invoke("flowpilot_capture_home_snapshot", {
				snapshot: snapshot ?? null,
				expectedOwner,
			});
			if (
				displayed &&
				snapshot?.profileId === displayed.profile_id &&
				!snapshot.editing
			) {
				const proposal = displayed;
				displayed = undefined;
				await invoke("flowpilot_resolve_home_proposal", {
					profileId: proposal.profile_id,
					candidateFingerprint: proposal.candidate_fingerprint,
				});
			}
		});
	synchronization = next;
	return next;
}

function presentProposal(proposal: NativeHomeProposal) {
	if (!matchesNativeSnapshotOwner(proposal.owner)) return;
	const surface = useAssistantSurface.getState().homeSurface;
	if (!surface) return;
	const snapshot = surface.getSnapshot();
	if (snapshot.profileId !== proposal.profile_id) return;
	if (snapshot.candidateFingerprint === proposal.candidate_fingerprint) {
		displayed = proposal;
		return;
	}
	const result = surface.stageLayout(proposal.layout, {
		expectedProfileId: proposal.profile_id,
		expectedFingerprint: proposal.expected_fingerprint,
	});
	if (result.status === "staged") {
		displayed = proposal;
		void syncNativeHomeSnapshot().catch(() => {});
	} else {
		toast.info("A FlowPilot Home proposal is waiting for review.", {
			description: result.message,
		});
	}
}

export function installNativeHomeAdapter(): () => void {
	let disposed = false;
	let removeListener: (() => void) | undefined;
	const synchronize = () => {
		void syncNativeHomeSnapshot().catch(() => {});
	};
	const restore = async () => {
		await syncNativeHomeSnapshot();
		const proposal = await invoke<NativeHomeProposal | null>(
			"flowpilot_get_home_proposal",
		);
		if (!disposed && proposal) presentProposal(proposal);
	};
	void listen<NativeHomeProposal>(
		"flowpilot:home-layout-staged",
		({ payload }) => {
			if (!disposed) presentProposal(payload);
		},
	).then((unlisten) => {
		if (disposed) unlisten();
		else removeListener = unlisten;
	});
	const unsubscribe = useAssistantSurface.subscribe((state, previous) => {
		if (state.homeSurface !== previous.homeSurface) {
			if (state.homeSurface) void restore().catch(() => {});
			else synchronize();
		}
	});
	window.addEventListener("flowpilot:home-snapshot-changed", synchronize);
	const heartbeat = window.setInterval(synchronize, 2_000);
	void restore().catch(() => {});
	return () => {
		disposed = true;
		removeListener?.();
		unsubscribe();
		window.clearInterval(heartbeat);
		window.removeEventListener("flowpilot:home-snapshot-changed", synchronize);
		void invoke("flowpilot_capture_home_snapshot", { snapshot: null }).catch(
			() => {},
		);
	};
}
