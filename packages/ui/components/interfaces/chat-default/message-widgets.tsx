"use client";

import { Maximize2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { cn } from "../../../lib";
import {
	A2UIRenderer,
	type A2UIServerMessage,
	type Surface,
	type SurfaceComponent,
} from "../../a2ui";
import { applyA2UIMessage } from "../../a2ui/apply-a2ui-message";
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
	const componentKey = useMemo(
		() => JSON.stringify(widget.component),
		[widget.component],
	);

	const [surface, setSurface] = useState<Surface>(() => buildSurface(widget));
	const [maximized, setMaximized] = useState(false);

	// Re-seed the local surface only when the pushed widget definition actually
	// changes, so live in-place a2ui updates are not wiped on every stream tick.
	// biome-ignore lint/correctness/useExhaustiveDependencies: keyed on the serialized component
	useEffect(() => {
		setSurface(buildSurface(widget));
	}, [componentKey]);

	const onA2UIMessage = useCallback((message: A2UIServerMessage) => {
		setSurface((prev) => applyA2UIMessage(prev, message));
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
			<div className="max-h-120 overflow-auto">{renderer}</div>

			<Dialog open={maximized} onOpenChange={setMaximized}>
				<DialogContent className="w-screen h-screen max-w-none! max-h-none! p-0 rounded-none top-[50%]! left-[50%]! translate-x-[-50%]! translate-y-[-50%]! flex flex-col">
					<DialogTitle className="sr-only">Widget</DialogTitle>
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
					appId={appId}
					boardId={boardId}
					eventId={eventId}
				/>
			))}
		</div>
	);
}
