"use client";

import type {
	CanvasSettings,
	SurfaceComponent,
} from "@flow-like/flow-like-ui/components/a2ui/types";
import { appBuildFingerprint } from "@flow-like/flow-like-ui/lib/app-build/fingerprint";
import {
	type AssistantWidgetSurface,
	useAssistantSurface,
} from "@flow-like/flow-like-ui/state/assistant-surface";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
	nativeSnapshotOwner,
	matchesNativeSnapshotOwner,
	type NativeSnapshotOwner,
} from "../lib/native-snapshot-owner";

interface WidgetProposal {
	owner: NativeSnapshotOwner;
	proposal_id: string;
	profile_id: string;
	run_id: string | null;
	conversation_id: string | null;
	expected_fingerprint: string;
	surface_id: string;
	app_id: string;
	components: SurfaceComponent[];
	canvas_settings?: CanvasSettings;
}
export function widgetSnapshot(surface: AssistantWidgetSurface | null) {
	if (!surface?.appId) return null;
	return {
		surfaceId: surface.surfaceId,
		kind: surface.kind,
		appId: surface.appId,
		...(surface.boardId ? { boardId: surface.boardId } : {}),
		...(surface.pageId ? { pageId: surface.pageId } : {}),
		...(surface.widgetId ? { widgetId: surface.widgetId } : {}),
		currentComponents: surface.currentComponents,
		...(surface.currentCanvasSettings
			? { currentCanvasSettings: surface.currentCanvasSettings }
			: {}),
		selectedComponentIds: surface.selectedComponentIds,
	};
}
let synchronization: Promise<void> = Promise.resolve();
let capturedProfile: string | undefined;
export function syncNativeWidgetSnapshot(): Promise<void> {
	const expectedOwner = nativeSnapshotOwner();
	const snapshot = structuredClone(
		widgetSnapshot(useAssistantSurface.getState().widgetSurface),
	);
	const next = synchronization
		.catch(() => {})
		.then(async () => {
			if (snapshot && !matchesNativeSnapshotOwner(expectedOwner))
				throw new Error(
					"The editor profile changed before its draft was captured.",
				);
			const receipt = await invoke<{ profile_id: string }>(
				"flowpilot_capture_widget_snapshot",
				{ snapshot, expectedOwner },
			);
			capturedProfile = receipt.profile_id;
		});
	synchronization = next;
	return next;
}
export function proposalMatchesSurface(proposal: WidgetProposal): boolean {
	const snapshot = widgetSnapshot(useAssistantSurface.getState().widgetSurface);
	return Boolean(
		snapshot &&
			matchesNativeSnapshotOwner(proposal.owner) &&
			capturedProfile === proposal.profile_id &&
			snapshot.surfaceId === proposal.surface_id &&
			snapshot.appId === proposal.app_id &&
			appBuildFingerprint("widget-surface-v1", snapshot) ===
				proposal.expected_fingerprint,
	);
}
export function installNativeWidgetAdapter(): () => void {
	let disposed = false;
	let unlisten: (() => void) | undefined;
	let displayed:
		| {
				proposal: WidgetProposal;
				pending: ReturnType<
					typeof useGlobalChatStore.getState
				>["pendingComponents"];
		  }
		| undefined;
	const present = async (proposal: WidgetProposal) => {
		await syncNativeWidgetSnapshot();
		if (disposed) return;
		const store = useGlobalChatStore.getState();
		if (
			proposal.conversation_id &&
			proposal.conversation_id !== store.activeConversationId
		)
			return;
		if (!proposalMatchesSurface(proposal)) {
			toast.info("A FlowPilot UI proposal is waiting for review.", {
				description:
					"The builder changed while the UI was generated. Your current draft is unchanged.",
			});
			return;
		}
		if (
			store.pendingComponents &&
			store.pendingComponentsOwnerRunId !== proposal.run_id
		)
			return;
		const pending = {
			components: proposal.components,
			canvasSettings: proposal.canvas_settings ?? undefined,
			surfaceId: proposal.surface_id,
			appId: proposal.app_id,
		};
		store.setPendingComponents(proposal.run_id, pending);
		displayed = { proposal, pending };
	};
	const restore = async () => {
		await syncNativeWidgetSnapshot();
		const proposal = await invoke<WidgetProposal | null>(
			"flowpilot_get_widget_proposal",
		);
		if (
			proposal &&
			!disposed &&
			displayed?.proposal.proposal_id !== proposal.proposal_id
		)
			await present(proposal);
	};
	void listen<WidgetProposal>(
		"flowpilot:widget-components-staged",
		({ payload }) => {
			void present(payload).catch(() => {});
		},
	).then((remove) => {
		if (disposed) remove();
		else unlisten = remove;
	});
	const unsubscribeSurface = useAssistantSurface.subscribe(
		(state, previous) => {
			if (state.widgetSurface !== previous.widgetSurface) {
				void syncNativeWidgetSnapshot().catch(() => {});
				if (
					state.widgetSurface &&
					state.widgetSurface.surfaceId !== previous.widgetSurface?.surfaceId
				)
					void restore().catch(() => {});
			}
		},
	);
	const unsubscribeChat = useGlobalChatStore.subscribe((state, previous) => {
		if (
			displayed &&
			state.pendingComponents !== displayed.pending &&
			previous.pendingComponents === displayed.pending
		) {
			const proposal = displayed.proposal;
			displayed = undefined;
			void invoke("flowpilot_resolve_widget_proposal", {
				proposalId: proposal.proposal_id,
			}).catch(() => {});
		}
		if (state.activeConversationId !== previous.activeConversationId)
			void restore().catch(() => {});
	});
	const heartbeat = window.setInterval(() => {
		void syncNativeWidgetSnapshot().catch(() => {});
	}, 2_000);
	void restore().catch(() => {});
	return () => {
		disposed = true;
		unlisten?.();
		unsubscribeSurface();
		unsubscribeChat();
		window.clearInterval(heartbeat);
		void invoke("flowpilot_capture_widget_snapshot", { snapshot: null }).catch(
			() => {},
		);
	};
}
