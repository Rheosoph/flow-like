"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ComponentPropsWithoutRef,
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import { UsePageContent } from "../../index";
import { inlineAppPageSnapshotAttribute } from "../../lib/app-page-snapshot";
import { EVENT_CONFIG } from "../../lib/event-config";
import {
	type InlineAppPage,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { PortalContainerProvider } from "../ui/portal-container";
import {
	type InlineAppPageTarget,
	getInlineAppPageTarget,
	observeInlineAppPageSlot,
	registerInlineAppPageRuntime,
	subscribeInlineAppPagePlacement,
} from "./inline-app-page-runtime-registry";

const INLINE_APP_PAGE_PRESENT_EVENT = "flowpilot:inline-app-page-present";

export interface InlineAppPagePresentDetail {
	appId: string;
	eventId?: string;
}

/** Ask the existing card to present a page without changing the page's live navigation target. */
export function presentInlineAppPage(appId: string, eventId?: string) {
	if (typeof window === "undefined") return;
	window.dispatchEvent(
		new CustomEvent<InlineAppPagePresentDetail>(INLINE_APP_PAGE_PRESENT_EVENT, {
			detail: { appId, eventId },
		}),
	);
}

export function subscribeToInlineAppPagePresentation(
	listener: (detail: InlineAppPagePresentDetail) => void,
): () => void {
	if (typeof window === "undefined") return () => {};
	const handlePresent = (event: Event) => {
		listener((event as CustomEvent<InlineAppPagePresentDetail>).detail);
	};
	window.addEventListener(INLINE_APP_PAGE_PRESENT_EVENT, handlePresent);
	return () =>
		window.removeEventListener(INLINE_APP_PAGE_PRESENT_EVENT, handlePresent);
}

export { getInlineAppPageTarget };

interface InlineAppPageSlotProps extends ComponentPropsWithoutRef<"div"> {
	pageId: string;
}

/** A visible destination for the stable page host owned by InlineAppPageRuntimeHost. */
export function InlineAppPageSlot({
	pageId,
	className,
	...props
}: InlineAppPageSlotProps) {
	const slotRef = useRef<HTMLDivElement>(null);

	useLayoutEffect(() => {
		const slot = slotRef.current;
		if (!slot) return;
		const observation = observeInlineAppPageSlot(pageId, slot);
		return observation.disconnect;
	}, [pageId]);

	return (
		<div
			ref={slotRef}
			data-flowpilot-page-slot={pageId}
			className={className}
			{...props}
		/>
	);
}

function configurePortalHost(
	host: HTMLDivElement,
	pageId: string,
	appId: string,
	eventId?: string,
): void {
	host.className =
		"relative flex h-full w-full min-w-0 flex-col overflow-hidden bg-background text-foreground contain-[layout_paint]";
	host.dataset.flowpilotPageRuntime = pageId;
	for (const [attribute, value] of Object.entries(
		inlineAppPageSnapshotAttribute(appId, eventId),
	)) {
		host.setAttribute(attribute, value);
	}
}

function InlineAppPageRuntime({ page }: { page: InlineAppPage }) {
	const { t } = useTranslation("chat");
	const parkingRef = useRef<HTMLDivElement>(null);
	const registrationRef = useRef<ReturnType<
		typeof registerInlineAppPageRuntime
	> | null>(null);
	const [portalHost, setPortalHost] = useState<HTMLDivElement | null>(null);
	const [target, setTarget] = useState<InlineAppPageTarget>({
		routePath: "/",
		eventId: page.eventId ?? null,
		queryParams: {},
	});
	const initialSnapshotEventIdRef = useRef(page.eventId);

	const handleNavigate = useCallback(
		(next: {
			routePath?: string | null;
			eventId?: string | null;
			queryParams?: Record<string, string>;
		}) => {
			setTarget((current) => ({
				routePath: next.routePath ?? current.routePath,
				eventId: next.eventId === undefined ? current.eventId : next.eventId,
				queryParams: next.queryParams ?? current.queryParams,
			}));
		},
		[],
	);
	const handleResolvedPage = useCallback(
		(next: { eventId: string }) => {
			useGlobalChatStore
				.getState()
				.retargetInlineAppPage(page.id, next.eventId);
		},
		[page.id],
	);

	useLayoutEffect(() => {
		if (typeof document === "undefined") return;
		const host = document.createElement("div");
		configurePortalHost(
			host,
			page.id,
			page.appId,
			initialSnapshotEventIdRef.current,
		);
		setPortalHost(host);
		return () => host.remove();
	}, [page.id, page.appId]);

	useLayoutEffect(() => {
		if (!portalHost) return;
		for (const [attribute, value] of Object.entries(
			inlineAppPageSnapshotAttribute(page.appId, page.eventId),
		)) {
			portalHost.setAttribute(attribute, value);
		}
	}, [page.appId, page.eventId, portalHost]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: target updates through the stable registration below; re-registering would move the host needlessly.
	useLayoutEffect(() => {
		const parkingSlot = parkingRef.current;
		if (!portalHost || !parkingSlot) return;

		const registration = registerInlineAppPageRuntime(
			page.id,
			portalHost,
			parkingSlot,
			target,
		);
		registrationRef.current = registration;
		return () => {
			if (registrationRef.current === registration) {
				registrationRef.current = null;
			}
			registration.unregister();
		};
	}, [page.id, portalHost]);

	useEffect(() => {
		registrationRef.current?.updateTarget(target);
	}, [target]);

	// A parked page is mounted but off screen. Its timed workflows must idle while it is
	// there, or a page opened once keeps billing a board run every interval for a surface
	// nobody can see.
	const [placed, setPlaced] = useState(false);
	useEffect(
		() => subscribeInlineAppPagePlacement(page.id, setPlaced),
		[page.id],
	);

	return (
		<>
			<div
				ref={parkingRef}
				data-flowpilot-page-parking={page.id}
				aria-hidden="true"
				inert
				style={{
					contain: "layout paint",
					height: "640px",
					left: "-100000px",
					overflow: "hidden",
					pointerEvents: "none",
					position: "fixed",
					top: 0,
					width: "960px",
				}}
			/>
			{portalHost &&
				createPortal(
					// Radix portals default to the top document's body, which puts a page's
					// dialogs, drawers, popovers and selects outside the host that moves and
					// parks: they cover the whole chat instead of the card, ignore its
					// containment, and stay on screen after the page has parked. Anchoring
					// them to the host keeps trigger and overlay in one moving subtree.
					<PortalContainerProvider container={portalHost}>
						<UsePageContent
							eventConfig={EVENT_CONFIG}
							notFound={
								<div className="flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground">
									{t(
										"thisAppPageIsNoLongerAvailable",
										"This app page is no longer available.",
									)}
								</div>
							}
							appId={page.appId}
							routePath={target.routePath}
							eventId={target.eventId}
							queryParams={target.queryParams}
							embedded
							eventIdTakesPrecedence
							active={placed}
							onNavigate={handleNavigate}
							onResolvedPage={handleResolvedPage}
						/>
					</PortalContainerProvider>,
					portalHost,
					page.id,
				)}
		</>
	);
}

/**
 * Owns every live inline page independently of the chat surface. Mount this once beside
 * GlobalToolBridge so hiding the overlay or collapsing a card parks pages instead of unloading
 * them.
 */
export function InlineAppPageRuntimeHost() {
	const pages = useGlobalChatStore((state) => state.inlineAppPages);
	return pages.map((page) => (
		<InlineAppPageRuntime key={page.id} page={page} />
	));
}
