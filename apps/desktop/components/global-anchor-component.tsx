"use client";

import { createId } from "@paralleldrive/cuid2";
import { isTauri as isTauriRuntime } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openUrl as shellOpen } from "@tauri-apps/plugin-opener";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "@tm9657/flow-like-ui";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

const isIosLike = () => {
	if (typeof navigator === "undefined") return false;
	// iPhone, iPad, iPod; also iPadOS reports MacIntel + touch
	return (
		/iPad|iPhone|iPod/.test(navigator.userAgent) ||
		(navigator.platform === "MacIntel" && (navigator as any).maxTouchPoints > 1)
	);
};

const isTauri = () => typeof window !== "undefined" && isTauriRuntime();

const isHttpish = (href: string) => /^(https?:|mailto:|tel:)/i.test(href);

const sameOrigin = (href: string) => {
	try {
		const u = new URL(href, location.href);
		return u.origin === location.origin;
	} catch {
		return false;
	}
};

const wantsExternal = (a: HTMLAnchorElement) =>
	a.getAttribute("target") === "_blank" ||
	a.rel.split(/\s+/).includes("external") ||
	a.dataset.openExternal === "true";

const resolveWindowUrl = (href: string) => {
	try {
		const parsed = new URL(href, location.href);
		return parsed.origin === location.origin
			? `${parsed.pathname}${parsed.search}${parsed.hash}`
			: parsed.toString();
	} catch {
		return href;
	}
};

interface NavigableInfo {
	href: string;
	title?: string;
	element: HTMLElement;
	isAnchor: boolean;
}

// Best-effort external opener that works on iOS; avoids blocking the user gesture.
const openInBrowser = async (href: string) => {
	try {
		await shellOpen(href);
		return true;
	} catch {
		try {
			// May still open in the same webview, but better than doing nothing
			window.open(href, "_blank", "noopener,noreferrer");
			return true;
		} catch {
			location.href = href; // last-resort fallback
			return true;
		}
	}
};

const GlobalAnchorHandler = () => {
	const [contextMenuData, setContextMenuData] = useState<{
		x: number;
		y: number;
		href: string;
		show: boolean;
		title?: string;
	} | null>(null);

	// Track touch/pointer start for iOS to differentiate scroll from tap
	const touchMetaRef = useRef<{
		x: number;
		y: number;
		time: number;
		active: boolean;
	}>({
		x: 0,
		y: 0,
		time: 0,
		active: false,
	});

	const IOS = useMemo(isIosLike, []);
	const TAURI = useMemo(isTauri, []);

	const createNewWindow = useCallback(
		(url: string, title?: string) => {
			// Desktop-only: iOS WKWebView doesn't support multiple windows like desktop
			if (!TAURI || IOS) {
				if (isHttpish(url)) void openInBrowser(url);
				return;
			}

			// True external links (different origin) should open in the system browser
			if (isHttpish(url) && !sameOrigin(url)) {
				void openInBrowser(url);
				return;
			}

			const resolvedUrl = resolveWindowUrl(url);

			const windowLabel = `window-${createId()}`;
			try {
				const webview = new WebviewWindow(windowLabel, {
					url: resolvedUrl,
					title: title ?? "Flow-Like",
					focus: true,
					resizable: true,
					maximized: true,
				});

				webview.once("tauri://error", (e) => {
					console.error("Failed to create new window:", e);
				});
			} catch (error) {
				console.error("Failed to create new window:", error);
			}
		},
		[IOS, TAURI],
	);

	useEffect(() => {
		const lastTouchHandledAt = { value: 0 };
		const findAnchorElement = (
			target: HTMLElement,
		): HTMLAnchorElement | null => {
			let el: HTMLElement | null = target;
			while (el) {
				if (el.tagName === "A") return el as HTMLAnchorElement;
				el = el.parentElement;
			}
			return null;
		};

		const findNavigable = (target: HTMLElement): NavigableInfo | null => {
			const anchor = findAnchorElement(target);
			if (anchor?.href) {
				return {
					href: anchor.href,
					title: anchor.textContent?.trim() ?? anchor.getAttribute("title") ?? undefined,
					element: anchor,
					isAnchor: true,
				};
			}
			let el: HTMLElement | null = target;
			while (el) {
				const dataHref = el.getAttribute("data-href");
				if (dataHref) {
					return {
						href: dataHref,
						title: el.getAttribute("data-title") ?? el.textContent?.trim() ?? undefined,
						element: el,
						isAnchor: false,
					};
				}
				el = el.parentElement;
			}
			return null;
		};

		// Thresholds to avoid accidental activations while scrolling
		const MOVE_THRESHOLD = 10; // px
		const TIME_THRESHOLD = 800; // ms (long press treated as non-tap here)

		const recordStart = (x: number, y: number) => {
			touchMetaRef.current = { x, y, time: Date.now(), active: true };
		};

		const isValidTap = (x: number, y: number) => {
			if (!touchMetaRef.current.active) return false;
			const dx = Math.abs(x - touchMetaRef.current.x);
			const dy = Math.abs(y - touchMetaRef.current.y);
			const dt = Date.now() - touchMetaRef.current.time;
			return dx < MOVE_THRESHOLD && dy < MOVE_THRESHOLD && dt < TIME_THRESHOLD;
		};

		// Unified external open handler
		const openExternallyIfNeeded = async (
			a: HTMLAnchorElement,
			e: MouseEvent,
		) => {
			const href = a.href;
			if (!href) return false;

			const externalIntent = wantsExternal(a);
			const httpish = isHttpish(href);
			const same = sameOrigin(href);

			// Rules:
			// - iOS: route http(s)/mailto/tel via shell when either true external OR link expresses external intent (_blank/rel=external)
			// - Desktop: target=_blank on any platform: if http(s)/mailto/tel and not same-origin app route, open via shell
			// - otherwise let normal navigation happen
			if (!httpish) return false;

			if (IOS) {
				if (externalIntent || !same) {
					e.preventDefault();
					e.stopPropagation();
					void openInBrowser(href);
					return true;
				}
				return false;
			}

			if (TAURI && externalIntent && !same) {
				e.preventDefault();
				e.stopPropagation();
				void openInBrowser(href);
				return true;
			}
			return false;
		};

		const handleMouseDown = (event: MouseEvent) => {
			if (event.button === 1) {
				const nav = findNavigable(event.target as HTMLElement);
				if (nav) {
					event.preventDefault();
					event.stopPropagation();
					event.stopImmediatePropagation();
				}
			}
		};

		const handleAuxClick = (event: MouseEvent) => {
			if (event.button !== 1) return;
			const nav = findNavigable(event.target as HTMLElement);
			if (!nav) return;

			event.preventDefault();
			event.stopPropagation();
			event.stopImmediatePropagation();

			if (IOS) {
				if (isHttpish(nav.href) && !sameOrigin(nav.href)) {
					void openInBrowser(nav.href);
				}
			} else {
				createNewWindow(nav.href, nav.title);
			}
		};

		const handleTouchStart = (event: TouchEvent) => {
			if (!IOS) return;
			const t = event.touches[0];
			if (!t) return;
			recordStart(t.clientX, t.clientY);
		};

		const handlePointerDown = (event: PointerEvent) => {
			if (!IOS) return;
			if (event.pointerType !== "touch" && event.pointerType !== "pen") return;
			recordStart(event.clientX, event.clientY);
		};

		const handleTouchEnd = async (event: TouchEvent) => {
			if (!IOS) return;
			const t = event.changedTouches[0];
			if (!t) return;
			const validTap = isValidTap(t.clientX, t.clientY);
			touchMetaRef.current.active = false;
			if (!validTap) return; // treat as scroll/drag
			const anchor = findAnchorElement(event.target as HTMLElement);
			if (!anchor?.href) return;
			const href = anchor.href;
			if (!isHttpish(href)) return;
			const externalIntent = wantsExternal(anchor);
			const same = sameOrigin(href);
			if (externalIntent || !same) {
				event.preventDefault();
				event.stopPropagation();
				void openInBrowser(href);
				lastTouchHandledAt.value = Date.now();
			}
		};

		const handlePointerUp = async (event: PointerEvent) => {
			if (!IOS) return;
			if (event.pointerType !== "touch" && event.pointerType !== "pen") return;
			const validTap = isValidTap(event.clientX, event.clientY);
			touchMetaRef.current.active = false;
			if (!validTap) return; // scrolling
			const anchor = findAnchorElement(event.target as HTMLElement);
			if (!anchor?.href) return;
			const href = anchor.href;
			if (!isHttpish(href)) return;
			const externalIntent = wantsExternal(anchor);
			const same = sameOrigin(href);
			if (externalIntent || !same) {
				event.preventDefault();
				event.stopPropagation();
				void openInBrowser(href);
				lastTouchHandledAt.value = Date.now();
			}
		};

		const handleClick = async (event: MouseEvent) => {
			if (IOS && Date.now() - lastTouchHandledAt.value < 500) {
				event.preventDefault();
				event.stopPropagation();
				event.stopImmediatePropagation?.();
				setContextMenuData(null);
				return;
			}

			// Check for data-href elements first (Cmd/Ctrl+Click support)
			const nav = findNavigable(event.target as HTMLElement);

			// Cmd/Ctrl+Click: open in new window (desktop standard)
			if (nav && (event.metaKey || event.ctrlKey) && TAURI && !IOS) {
				event.preventDefault();
				event.stopPropagation();
				event.stopImmediatePropagation?.();
				createNewWindow(nav.href, nav.title);
				setContextMenuData(null);
				return;
			}

			const anchor = findAnchorElement(event.target as HTMLElement);
			if (!anchor?.href) {
				setContextMenuData(null);
				return;
			}

			const handled = await openExternallyIfNeeded(anchor, event);
			if (handled) {
				event.stopImmediatePropagation?.();
				setContextMenuData(null);
				return;
			}

			if (wantsExternal(anchor) && TAURI && !IOS) {
				event.preventDefault();
				event.stopPropagation();
				const linkTitle =
					anchor.textContent?.trim() ??
					anchor.getAttribute("title") ??
					undefined;
				createNewWindow(anchor.href, linkTitle);
			}

			setContextMenuData(null);
		};

		const handleContextMenu = (event: MouseEvent) => {
			const nav = findNavigable(event.target as HTMLElement);
			if (!nav) return;

			event.preventDefault();

			setContextMenuData({
				x: event.clientX,
				y: event.clientY,
				href: nav.href,
				title: nav.title,
				show: true,
			});
		};

		document.addEventListener("mousedown", handleMouseDown, true);
		document.addEventListener("auxclick", handleAuxClick, true);
		document.addEventListener("touchstart", handleTouchStart, {
			passive: true,
			capture: true,
		});
		document.addEventListener("pointerdown", handlePointerDown as any, true);
		document.addEventListener("touchend", handleTouchEnd, true);
		document.addEventListener("pointerup", handlePointerUp as any, true);
		document.addEventListener("click", handleClick, true);
		document.addEventListener("contextmenu", handleContextMenu, true);

		return () => {
			document.removeEventListener("mousedown", handleMouseDown, true);
			document.removeEventListener("auxclick", handleAuxClick, true);
			document.removeEventListener("touchstart", handleTouchStart, true);
			document.removeEventListener(
				"pointerdown",
				handlePointerDown as any,
				true,
			);
			document.removeEventListener("touchend", handleTouchEnd, true);
			document.removeEventListener("pointerup", handlePointerUp as any, true);
			document.removeEventListener("click", handleClick, true);
			document.removeEventListener("contextmenu", handleContextMenu, true);
		};
	}, [IOS, TAURI, createNewWindow]);

	return (
		<>
			{contextMenuData && (
				<div
					style={{
						position: "fixed",
						left: contextMenuData.x,
						top: contextMenuData.y,
						zIndex: 50,
						pointerEvents: "auto",
					}}
				>
					<DropdownMenu
						open={contextMenuData.show}
						onOpenChange={(open) => {
							if (!open) setContextMenuData(null);
						}}
					>
						<DropdownMenuTrigger asChild>
							<div className="w-1 h-1 opacity-0" />
						</DropdownMenuTrigger>

						<DropdownMenuContent side="bottom" align="start">
							<DropdownMenuItem asChild>
								<button
									className="w-full"
									onMouseDown={async (e) => {
										e.preventDefault();
										e.stopPropagation();
										const href = contextMenuData.href;
										const title = contextMenuData.title;

										// iOS: always open in browser; Desktop: spawn new window for same-origin/_blank, shell for true external
										if (isIosLike()) {
											if (isHttpish(href)) {
												await openInBrowser(href);
											}
										} else if (isHttpish(href) && !sameOrigin(href)) {
											await openInBrowser(href);
										} else {
											createNewWindow(href, title);
										}
										setContextMenuData(null);
									}}
									style={{ cursor: "pointer" }}
								>
									{IOS ? "Open in browser" : "Open in new window"}
								</button>
							</DropdownMenuItem>

							<DropdownMenuItem asChild>
								<button
									className="w-full"
									onMouseDown={(e) => {
										e.preventDefault();
										e.stopPropagation();
										navigator.clipboard.writeText(contextMenuData.href);
										setContextMenuData(null);
									}}
									style={{ cursor: "pointer" }}
								>
									Copy Link
								</button>
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</div>
			)}
		</>
	);
};

export default GlobalAnchorHandler;
