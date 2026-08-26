export interface InlineAppPageTarget {
	routePath: string;
	eventId: string | null;
	queryParams: Record<string, string>;
}

function cloneTarget(target: InlineAppPageTarget): InlineAppPageTarget {
	return { ...target, queryParams: { ...target.queryParams } };
}

interface RuntimeRegistration {
	token: symbol;
	host: HTMLElement;
	parkingSlot: HTMLElement;
	target: InlineAppPageTarget;
	placed: boolean;
}

interface SlotRegistration {
	token: symbol;
	element: HTMLElement;
	order: number;
}

const runtimes = new Map<string, RuntimeRegistration>();
const slots = new Map<string, Map<symbol, SlotRegistration>>();
let slotOrder = 0;

function activeSlot(pageId: string): SlotRegistration | undefined {
	const registrations = slots.get(pageId);
	if (!registrations) return undefined;

	let latest: SlotRegistration | undefined;
	for (const registration of registrations.values()) {
		if (!latest || registration.order > latest.order) latest = registration;
	}
	return latest;
}

function rememberVisibleSize(
	runtime: RuntimeRegistration,
	visibleSlot: HTMLElement,
) {
	const hostRect = runtime.host.getBoundingClientRect();
	const slotRect = visibleSlot.getBoundingClientRect();
	const width = hostRect.width || slotRect.width;
	const height = hostRect.height || slotRect.height;
	if (width > 0) runtime.parkingSlot.style.width = `${width}px`;
	if (height > 0) runtime.parkingSlot.style.height = `${height}px`;
}

interface ScrollOffset {
	element: Element;
	top: number;
	left: number;
}

interface MoveSnapshot {
	scrollOffsets: ScrollOffset[];
	focused: HTMLElement | null;
	selection: { start: number; end: number } | null;
}

function isTextEntry(
	element: HTMLElement,
): element is HTMLInputElement | HTMLTextAreaElement {
	return typeof (element as HTMLInputElement).selectionStart === "number";
}

/**
 * Everything React owns rides through a host move untouched, because the portal is never
 * torn down. Scroll offsets and focus do not: the browser drops both the moment the subtree
 * is disconnected, so a collapsed-and-reopened card would otherwise jump to the top of the
 * page and drop the caret out of whatever field the user was filling in.
 */
function captureMoveState(host: HTMLElement): MoveSnapshot {
	const scrollOffsets: ScrollOffset[] = [];
	const record = (element: Element) => {
		if (element.scrollTop === 0 && element.scrollLeft === 0) return;
		scrollOffsets.push({
			element,
			top: element.scrollTop,
			left: element.scrollLeft,
		});
	};
	record(host);
	host.querySelectorAll("*").forEach(record);

	const active = host.ownerDocument.activeElement;
	const focused =
		active instanceof HTMLElement && host.contains(active) ? active : null;
	const selection =
		focused && isTextEntry(focused)
			? {
					start: focused.selectionStart ?? 0,
					end: focused.selectionEnd ?? 0,
				}
			: null;

	return { scrollOffsets, focused, selection };
}

function restoreMoveState(snapshot: MoveSnapshot, parked: boolean) {
	for (const { element, top, left } of snapshot.scrollOffsets) {
		element.scrollTop = top;
		element.scrollLeft = left;
	}

	// Focus must not be taken back into a parked subtree: it is `inert`, and pulling the
	// caret out of whatever the user is typing in now would be worse than losing it.
	if (parked || !snapshot.focused) return;
	snapshot.focused.focus({ preventScroll: true });
	if (!snapshot.selection || !isTextEntry(snapshot.focused)) return;
	try {
		snapshot.focused.setSelectionRange(
			snapshot.selection.start,
			snapshot.selection.end,
		);
	} catch {
		// Input types like `number` and `email` reject range selection; the focus still landed.
	}
}

function placeRuntime(pageId: string) {
	const runtime = runtimes.get(pageId);
	if (!runtime) return;

	const slot = activeSlot(pageId);
	const destination = slot?.element ?? runtime.parkingSlot;
	if (runtime.host.parentElement === destination) return;

	if (!slot && runtime.host.parentElement) {
		rememberVisibleSize(runtime, runtime.host.parentElement);
	}
	const snapshot = captureMoveState(runtime.host);
	destination.appendChild(runtime.host);

	const parked = destination === runtime.parkingSlot;
	runtime.host.dataset.flowpilotPageRuntimeState = parked
		? "parked"
		: "visible";
	runtime.host.toggleAttribute("inert", parked);
	if (parked) runtime.host.setAttribute("aria-hidden", "true");
	else runtime.host.removeAttribute("aria-hidden");

	restoreMoveState(snapshot, parked);
	setPlaced(runtime, pageId, !parked);
}

const placementListeners = new Map<string, Set<(placed: boolean) => void>>();

function setPlaced(
	runtime: RuntimeRegistration,
	pageId: string,
	placed: boolean,
) {
	if (runtime.placed === placed) return;
	runtime.placed = placed;
	const listeners = placementListeners.get(pageId);
	if (!listeners) return;
	for (const listener of listeners) listener(placed);
}

/** Whether this runtime currently sits in a visible slot rather than the parking slot. */
export function isInlineAppPagePlaced(pageId: string): boolean {
	return runtimes.get(pageId)?.placed ?? false;
}

/**
 * Observe whether a runtime is on screen. A parked page stays mounted and keeps running —
 * timers included — so anything that should idle while nobody is looking needs the signal.
 * The listener is called once with the current placement on subscribe.
 */
export function subscribeInlineAppPagePlacement(
	pageId: string,
	listener: (placed: boolean) => void,
): () => void {
	const listeners = placementListeners.get(pageId) ?? new Set();
	listeners.add(listener);
	placementListeners.set(pageId, listeners);
	listener(isInlineAppPagePlaced(pageId));

	return () => {
		const current = placementListeners.get(pageId);
		if (!current?.delete(listener)) return;
		if (current.size === 0) placementListeners.delete(pageId);
	};
}

export interface InlineAppPageRuntimeRegistration {
	updateTarget: (target: InlineAppPageTarget) => void;
	unregister: () => void;
}

/**
 * Register one stable DOM host for a live page. The host is moved between card slots and the
 * parking slot as a DOM node, so the React portal rendered into it never remounts.
 */
export function registerInlineAppPageRuntime(
	pageId: string,
	host: HTMLElement,
	parkingSlot: HTMLElement,
	initialTarget: InlineAppPageTarget,
): InlineAppPageRuntimeRegistration {
	const token = Symbol(pageId);
	const registration: RuntimeRegistration = {
		token,
		host,
		parkingSlot,
		target: cloneTarget(initialTarget),
		placed: false,
	};
	runtimes.set(pageId, registration);
	placeRuntime(pageId);

	return {
		updateTarget: (target) => {
			const current = runtimes.get(pageId);
			if (current?.token !== token) return;
			current.target = cloneTarget(target);
		},
		unregister: () => {
			const current = runtimes.get(pageId);
			if (current?.token !== token) {
				host.remove();
				return;
			}
			setPlaced(current, pageId, false);
			runtimes.delete(pageId);
			host.remove();
		},
	};
}

/** Register a visible destination for a page runtime. The most recently mounted slot wins. */
export function registerInlineAppPageSlot(
	pageId: string,
	element: HTMLElement,
): () => void {
	const token = Symbol(pageId);
	const registration: SlotRegistration = {
		token,
		element,
		order: ++slotOrder,
	};
	const pageSlots = slots.get(pageId) ?? new Map();
	pageSlots.set(token, registration);
	slots.set(pageId, pageSlots);
	placeRuntime(pageId);

	return () => {
		const currentSlots = slots.get(pageId);
		if (!currentSlots?.delete(token)) return;
		if (currentSlots.size === 0) slots.delete(pageId);
		placeRuntime(pageId);
	};
}

function hasVisibleLayout(element: HTMLElement): boolean {
	if (!element.isConnected) return false;
	const rect = element.getBoundingClientRect();
	if (rect.width <= 0 || rect.height <= 0) return false;

	const view = element.ownerDocument.defaultView;
	let current: HTMLElement | null = element;
	while (current) {
		const style = view?.getComputedStyle(current);
		if (
			current.hidden ||
			style?.display === "none" ||
			style?.visibility === "hidden" ||
			style?.visibility === "collapse" ||
			style?.getPropertyValue("content-visibility") === "hidden"
		) {
			return false;
		}
		current = current.parentElement;
	}
	return true;
}

export interface InlineAppPageSlotObservation {
	/** Re-evaluate synchronously. Exposed so callers and tests can react to non-observable layout changes. */
	refresh: () => void;
	disconnect: () => void;
}

/**
 * Keep a card slot registered only while it has a visible layout box. A mounted Chat can sit below
 * `display: none` while another workspace is shown; treating that slot as visible would strand the
 * live page in a zero-sized subtree instead of its capture-safe parking slot.
 */
export function observeInlineAppPageSlot(
	pageId: string,
	element: HTMLElement,
): InlineAppPageSlotObservation {
	let unregister: (() => void) | undefined;
	const refresh = () => {
		const visible = hasVisibleLayout(element);
		if (visible && !unregister) {
			unregister = registerInlineAppPageSlot(pageId, element);
		} else if (!visible && unregister) {
			unregister();
			unregister = undefined;
		}
	};

	const view = element.ownerDocument.defaultView;
	const ResizeObserverConstructor = view?.ResizeObserver;
	const resizeObserver = ResizeObserverConstructor
		? new ResizeObserverConstructor(refresh)
		: undefined;
	const MutationObserverConstructor = view?.MutationObserver;
	const mutationObserver = MutationObserverConstructor
		? new MutationObserverConstructor(refresh)
		: undefined;

	let current: HTMLElement | null = element;
	while (current) {
		resizeObserver?.observe(current);
		mutationObserver?.observe(current, {
			attributeFilter: ["class", "style", "hidden"],
			attributes: true,
		});
		current = current.parentElement;
	}
	view?.addEventListener("resize", refresh);
	refresh();

	return {
		refresh,
		disconnect: () => {
			resizeObserver?.disconnect();
			mutationObserver?.disconnect();
			view?.removeEventListener("resize", refresh);
			unregister?.();
			unregister = undefined;
		},
	};
}

export function getInlineAppPageTarget(
	pageId: string,
): InlineAppPageTarget | undefined {
	const target = runtimes.get(pageId)?.target;
	return target ? cloneTarget(target) : undefined;
}
