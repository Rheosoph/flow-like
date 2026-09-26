"use client";

import { useTranslation } from "@flow-like/locales";
import Maximize2 from "lucide-react/dist/esm/icons/maximize-2.js";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	registerWidgetSnapshotSource,
	scheduleWidgetSnapshot,
	unregisterWidgetSnapshotSource,
	widgetSnapshotAttribute,
} from "../../../lib/widget-snapshot";
import { A2UIRenderer } from "../../a2ui/A2UIRenderer";
import {
	applyA2UIMessage,
	normalizeA2UIWireMessage,
} from "../../a2ui/apply-a2ui-message";
import type {
	A2UIServerMessage,
	Surface,
	SurfaceComponent,
} from "../../a2ui/types";
import { Dialog, DialogContent, DialogTitle } from "../../ui/dialog";
import type { IChatWidget } from "./chat-db";

function buildSurface(widget: IChatWidget): Surface {
	const componentId = widget.instance_id;
	return {
		id: widget.surface_id || widget.instance_id,
		rootComponentId: componentId,
		components: {
			[componentId]: {
				id: componentId,
				component: widget.component as unknown as SurfaceComponent["component"],
			},
		},
	};
}

interface MessageWidgetProps {
	widget: IChatWidget;
	appId?: string;
	boardId?: string;
	eventId?: string;
	/** Pre-capture the rendered widget for the model's context once it settles. */
	snapshots: boolean;
}

function replaySurface(widget: IChatWidget): Surface {
	let next = buildSurface(widget);
	for (const update of widget.updates ?? []) {
		next = applyA2UIMessage(next, normalizeA2UIWireMessage(update));
	}
	return next;
}

function updateSignature(
	updates: unknown[] | undefined,
	index: number,
): string | null {
	if (!updates || index < 0 || index >= updates.length) return null;
	return JSON.stringify(updates[index]);
}

interface ReplayState {
	instanceId: string;
	appliedCount: number;
	/** The last applied entry itself; identity short-circuits the content check. */
	lastApplied: unknown;
	lastSig: string | null;
	/** Unpersisted action-feedback messages applied on top of the replay. */
	feedbackCount: number;
}

function replayState(widget: IChatWidget): ReplayState {
	const updates = widget.updates ?? [];
	return {
		instanceId: widget.instance_id,
		appliedCount: updates.length,
		lastApplied: updates[updates.length - 1],
		lastSig: updateSignature(updates, updates.length - 1),
		feedbackCount: 0,
	};
}

/** True when `updates` still starts with everything the replay already applied. */
function extendsReplay(state: ReplayState, updates: unknown[]): boolean {
	if (updates.length < state.appliedCount) return false;
	const index = state.appliedCount - 1;
	if (index < 0) return true;
	return (
		updates[index] === state.lastApplied ||
		updateSignature(updates, index) === state.lastSig
	);
}

/** Keys the snapshot cache: changes exactly when the rendered state does. */
function contentSignature(state: ReplayState): string {
	return `${state.feedbackCount}:${state.appliedCount}:${state.lastSig ?? ""}`;
}

/**
 * Renders a single embedded a2ui widget instance inside a chat message. The
 * widget is mounted in its own local surface so that action-feedback a2ui
 * updates (streamed back after a widget action triggers its workflow) mutate
 * this widget in place. A maximize control opens the same live surface in a
 * fullscreen dialog.
 *
 * Memoized on widget identity: the streaming bubble re-renders once per
 * animation frame while text arrives, and without this boundary every frame
 * re-rendered the whole widget tree (charts, maps, tables, micro-widget
 * iframes). The event processor and `mergeChatWidgets` keep the widget object
 * unless its updates actually grow, so identity is the right key.
 */
const MessageWidget = memo(function MessageWidget({
	widget,
	appId,
	boardId,
	eventId,
	snapshots,
}: MessageWidgetProps) {
	const { t } = useTranslation("chat");
	// Dexie liveQuery re-materializes message objects on every table write, so
	// object identity is NOT stable across renders of unchanged widgets. The
	// replay is applied incrementally: as long as the updates array extends
	// what was already applied (checked via the last applied entry's content,
	// not identity), only the new tail is applied onto the CURRENT surface —
	// full O(n) reseeds per streamed update become O(1), and unpersisted
	// action-feedback state (applied via onA2UIMessage) survives. The push-time
	// component snapshot never changes for an instance (re-registrations ride
	// the updates array), so a shrunk or diverged updates array is the only
	// full-reseed trigger.
	const replayRef = useRef<ReplayState>({
		instanceId: "",
		appliedCount: 0,
		lastApplied: undefined,
		lastSig: null,
		feedbackCount: 0,
	});
	const [surface, setSurface] = useState<Surface>(() => {
		replayRef.current = replayState(widget);
		return replaySurface(widget);
	});
	const [signature, setSignature] = useState(() =>
		contentSignature(replayRef.current),
	);
	const [maximized, setMaximized] = useState(false);
	const containerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const updates = widget.updates ?? [];
		const state = replayRef.current;

		if (
			state.instanceId === widget.instance_id &&
			extendsReplay(state, updates)
		) {
			if (updates.length === state.appliedCount) return;
			const tail = updates.slice(state.appliedCount);
			setSurface((prev) =>
				tail.reduce<Surface>(
					(acc, update) =>
						applyA2UIMessage(acc, normalizeA2UIWireMessage(update)),
					prev,
				),
			);
			state.appliedCount = updates.length;
			state.lastApplied = updates[updates.length - 1];
			state.lastSig = updateSignature(updates, updates.length - 1);
			setSignature(contentSignature(state));
			return;
		}

		replayRef.current = replayState(widget);
		setSurface(replaySurface(widget));
		setSignature(contentSignature(replayRef.current));
	}, [widget]);

	const onA2UIMessage = useCallback((message: A2UIServerMessage) => {
		setSurface((prev) =>
			applyA2UIMessage(prev, normalizeA2UIWireMessage(message)),
		);
		replayRef.current.feedbackCount += 1;
		setSignature(contentSignature(replayRef.current));
	}, []);

	// The inline container is the capture source; while maximized it is empty,
	// so pre-capture pauses and the last inline capture keeps serving. With
	// snapshots disabled the send path never asks for one, so rasterizing on
	// every settle would be pure main-thread waste.
	useEffect(() => {
		if (!snapshots) return;
		const instanceId = widget.instance_id;
		registerWidgetSnapshotSource(instanceId, signature);
		if (!maximized) {
			scheduleWidgetSnapshot(instanceId, signature, () => containerRef.current);
		}
		return () => unregisterWidgetSnapshotSource(instanceId);
	}, [widget.instance_id, signature, maximized, snapshots]);

	const renderer = (
		<A2UIRenderer
			surface={surface}
			appId={appId}
			boardId={boardId}
			eventId={eventId}
			isPreviewMode={true}
			onA2UIMessage={onA2UIMessage}
			className="w-full"
		/>
	);

	return (
		<div className="relative rounded-xl border bg-muted/20 overflow-hidden group/widget">
			<button
				type="button"
				onClick={() => setMaximized(true)}
				title="Maximize"
				className="absolute top-2 right-2 z-10 flex items-center justify-center rounded-md border bg-background/80 backdrop-blur-sm p-1.5 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover/widget:opacity-100 focus-visible:opacity-100"
			>
				<Maximize2 className="w-3.5 h-3.5" />
			</button>
			<div
				ref={containerRef}
				className="max-h-120 overflow-auto"
				{...widgetSnapshotAttribute(widget.instance_id)}
			>
				{/* Only one live renderer at a time: two mounted trees for the same
				    surface duplicate iframes, charts and map instances. */}
				{!maximized && renderer}
			</div>

			<Dialog open={maximized} onOpenChange={setMaximized}>
				<DialogContent className="w-screen h-screen max-w-none! max-h-none! p-0 rounded-none top-[50%]! left-[50%]! translate-x-[-50%]! translate-y-[-50%]! flex flex-col">
					<DialogTitle className="sr-only">{t("widget", "Widget")}</DialogTitle>
					<div className="flex-1 overflow-auto p-4">{renderer}</div>
				</DialogContent>
			</Dialog>
		</div>
	);
});

export interface MessageWidgetsProps {
	widgets: IChatWidget[] | undefined;
	appId?: string;
	boardId?: string;
	eventId?: string;
	className?: string;
	/** Pre-capture widgets for the model's context; off when the chat never attaches snapshots. */
	snapshots?: boolean;
}

export const MessageWidgets = memo(function MessageWidgets({
	widgets,
	appId,
	boardId,
	eventId,
	className,
	snapshots = true,
}: MessageWidgetsProps) {
	if (!widgets?.length) return null;

	return (
		<div
			className={cn("mt-2 flex flex-col gap-2 max-w-full w-full", className)}
		>
			{widgets.map((widget) => (
				<MessageWidget
					key={widget.instance_id}
					widget={widget}
					appId={widget.origin?.appId ?? appId}
					boardId={widget.origin?.boardId ?? boardId}
					eventId={widget.origin?.eventId ?? eventId}
					snapshots={snapshots}
				/>
			))}
		</div>
	);
});
