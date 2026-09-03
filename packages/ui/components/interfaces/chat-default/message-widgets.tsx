"use client";

import { useTranslation } from "@flow-like/locales";
import Maximize2 from "lucide-react/dist/esm/icons/maximize-2.js";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { widgetSnapshotAttribute } from "../../../lib/widget-snapshot";
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
}

/**
 * Renders a single embedded a2ui widget instance inside a chat message. The
 * widget is mounted in its own local surface so that action-feedback a2ui
 * updates (streamed back after a widget action triggers its workflow) mutate
 * this widget in place. A maximize control opens the same live surface in a
 * fullscreen dialog.
 */
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

function MessageWidget({
	widget,
	appId,
	boardId,
	eventId,
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
	const replayRef = useRef({
		instanceId: "",
		appliedCount: 0,
		lastSig: null as string | null,
	});
	const [surface, setSurface] = useState<Surface>(() => {
		const updates = widget.updates ?? [];
		replayRef.current = {
			instanceId: widget.instance_id,
			appliedCount: updates.length,
			lastSig: updateSignature(updates, updates.length - 1),
		};
		return replaySurface(widget);
	});
	const [maximized, setMaximized] = useState(false);

	useEffect(() => {
		const updates = widget.updates ?? [];
		const state = replayRef.current;
		const extendsApplied =
			state.instanceId === widget.instance_id &&
			updates.length >= state.appliedCount &&
			updateSignature(updates, state.appliedCount - 1) === state.lastSig;

		if (extendsApplied) {
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
			state.lastSig = updateSignature(updates, updates.length - 1);
			return;
		}

		replayRef.current = {
			instanceId: widget.instance_id,
			appliedCount: updates.length,
			lastSig: updateSignature(updates, updates.length - 1),
		};
		setSurface(replaySurface(widget));
	}, [widget]);

	const onA2UIMessage = useCallback((message: A2UIServerMessage) => {
		setSurface((prev) =>
			applyA2UIMessage(prev, normalizeA2UIWireMessage(message)),
		);
	}, []);

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
}

export interface MessageWidgetsProps {
	widgets: IChatWidget[] | undefined;
	appId?: string;
	boardId?: string;
	eventId?: string;
	className?: string;
}

export function MessageWidgets({
	widgets,
	appId,
	boardId,
	eventId,
	className,
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
				/>
			))}
		</div>
	);
}
