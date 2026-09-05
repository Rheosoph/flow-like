"use client";

import { useState } from "react";
import { USE_EVENT_CONFIG } from "../../../lib/event-config-use";
import { UsePageContent } from "../../interfaces/use-page-content";
import { PortalContainerProvider } from "../../ui/portal-container";
import type { HomeEmbedTarget } from "./config";

export default function HomeAppEmbedRuntime({
	target,
	active,
	onNavigate,
}: {
	target: HomeEmbedTarget;
	active: boolean;
	onNavigate: (next: {
		routePath?: string | null;
		eventId?: string | null;
		queryParams?: Record<string, string>;
	}) => void;
}) {
	const [container, setContainer] = useState<HTMLDivElement | null>(null);
	return (
		<div
			ref={setContainer}
			className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden [contain:layout_paint]"
		>
			<PortalContainerProvider container={container}>
				<UsePageContent
					eventConfig={USE_EVENT_CONFIG}
					appId={target.appId}
					routePath={target.routePath}
					eventId={target.eventId}
					queryParams={target.queryParams}
					embedded
					eventIdTakesPrecedence
					active={active}
					onNavigate={onNavigate}
				/>
			</PortalContainerProvider>
		</div>
	);
}
