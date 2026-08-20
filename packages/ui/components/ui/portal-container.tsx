"use client";

import { createContext, useContext } from "react";

const PortalContainerContext = createContext<HTMLElement | null>(null);

/**
 * Redirects portalled overlays (select menus, popovers, dialogs, tooltips) into a specific
 * element instead of the top document's body.
 *
 * The responsive preview renders the surface into a same-origin iframe, and Radix defaults
 * every portal to `globalThis.document.body` — the *host* document. An overlay landing there
 * is anchored to a trigger in another document: it is positioned against the wrong viewport,
 * its dismissal listeners watch the wrong document, and focus crossing the frame boundary
 * blurs the host window, which Radix's select treats as "close now". Providing the frame's
 * mount node keeps trigger and overlay in one document.
 */
export function PortalContainerProvider({
	container,
	children,
}: {
	container: HTMLElement | null;
	children: React.ReactNode;
}) {
	return (
		<PortalContainerContext.Provider value={container}>
			{children}
		</PortalContainerContext.Provider>
	);
}

/** The container portals should mount into, or undefined to keep the Radix default. */
export function usePortalContainer(): HTMLElement | undefined {
	return useContext(PortalContainerContext) ?? undefined;
}
