"use client";

import { useCallback, useEffect, useRef } from "react";
import { normalizeBoardVersion } from "../../lib/schema/flow/board-version";
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { useSurfaceManager } from "../a2ui/SurfaceManager";
import type { A2UIClientMessage, A2UIServerMessage } from "../a2ui/types";
import type { IUseInterfaceProps } from "./interfaces";

export function A2UIInterface({
	appId,
	event,
	config,
	toolbarRef,
	sidebarRef,
}: IUseInterfaceProps) {
	const { surfaces, handleServerMessage, getAllSurfaces } = useSurfaceManager();
	const streamRef = useRef<EventSource | null>(null);

	const handleClientMessage = useCallback(
		(message: A2UIClientMessage) => {
			if (config?.streamUrl) {
				fetch(config.streamUrl, {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(message),
				}).catch(console.error);
			}
		},
		[config?.streamUrl],
	);

	useEffect(() => {
		if (!config?.streamUrl) return;

		const eventSource = new EventSource(config.streamUrl);
		streamRef.current = eventSource;

		eventSource.onmessage = (evt) => {
			try {
				const message = JSON.parse(evt.data) as A2UIServerMessage;
				handleServerMessage(message);
			} catch (e) {
				console.error("Failed to parse A2UI message:", e);
			}
		};

		eventSource.onerror = () => {
			eventSource.close();
		};

		return () => {
			eventSource.close();
			streamRef.current = null;
		};
	}, [config?.streamUrl, handleServerMessage]);

	const allSurfaces = getAllSurfaces();

	if (allSurfaces.length === 0) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>Waiting for UI...</p>
			</div>
		);
	}

	return (
		<div className="h-full w-full overflow-auto">
			{allSurfaces.map((surface) => (
				<A2UIRenderer
					key={surface.id}
					surface={surface}
					onMessage={handleClientMessage}
					className="w-full min-h-full"
					appId={appId}
					boardId={event.board_id}
					boardVersion={normalizeBoardVersion(event.board_version)}
					eventId={event.id}
					isPreviewMode={true}
				/>
			))}
		</div>
	);
}

export function useA2UIInterface(props: IUseInterfaceProps) {
	return <A2UIInterface {...props} />;
}
