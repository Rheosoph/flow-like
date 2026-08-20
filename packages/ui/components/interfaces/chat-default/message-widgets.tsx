"use client";

import { useTranslation } from "@flow-like/locales";
import { Maximize2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { cn } from "../../../lib";
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
import { Dialog, DialogContent, DialogTitle } from "../../ui";
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
function MessageWidget({
	widget,
	appId,
	boardId,
	eventId,
}: MessageWidgetProps) {
	const { t } = useTranslation("chat");
	// Content keys: Dexie liveQuery re-materializes message objects on every
	// table write, so object identity is NOT stable across renders of unchanged
	// widgets — string equality is. Reseeding only on real content change keeps
	// unpersisted action-feedback state (applied via onA2UIMessage) alive.
	const componentKey = useMemo(
		() => JSON.stringify(widget.component),
		[widget.component],
	);
	const updatesKey = useMemo(
		() => JSON.stringify(widget.updates ?? []),
		[widget.updates],
	);

	// Snapshot + ordered replay of the widget's a2ui updates (attached by the
	// backend for pre-push updates, appended by the stream reducer for live
	// ones).
	// biome-ignore lint/correctness/useExhaustiveDependencies: keyed on serialized content, not object identity
	const seededSurface = useMemo(() => {
		let next = buildSurface(widget);
		for (const update of widget.updates ?? []) {
			next = applyA2UIMessage(next, normalizeA2UIWireMessage(update));
		}
		return next;
	}, [componentKey, updatesKey, widget.surface_id, widget.instance_id]);

	const [surface, setSurface] = useState<Surface>(seededSurface);
	const [maximized, setMaximized] = useState(false);

	useEffect(() => {
		setSurface(seededSurface);
	}, [seededSurface]);

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
				{renderer}
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
