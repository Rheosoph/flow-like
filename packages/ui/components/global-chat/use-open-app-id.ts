"use client";

import { usePathname } from "next/navigation";
import { useAssistantSurface } from "../../state/assistant-surface";

/** The app the person has open behind the chat, from the live surfaces or the `/use` route. */
export function useOpenAppId(): string | undefined {
	const pathname = usePathname();
	const surfaceAppId = useAssistantSurface(
		(state) =>
			state.boardSurface?.appId ??
			state.widgetSurface?.appId ??
			state.dataStudioSurface?.appId,
	);
	if (surfaceAppId) return surfaceAppId;
	if (pathname === "/use")
		return new URLSearchParams(window.location.search).get("id") || undefined;
	return undefined;
}
