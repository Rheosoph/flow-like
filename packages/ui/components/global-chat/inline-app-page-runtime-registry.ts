export interface InlineAppPageTarget {
	routePath: string;
	eventId: string | null;
}

interface RuntimeRegistration {
	token: symbol;
	host: HTMLElement;
	parkingSlot: HTMLElement;
	target: InlineAppPageTarget;
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

function placeRuntime(pageId: string) {
	const runtime = runtimes.get(pageId);
	if (!runtime) return;

	const slot = activeSlot(pageId);
	const destination = slot?.element ?? runtime.parkingSlot;
	if (runtime.host.parentElement === destination) return;

	if (!slot && runtime.host.parentElement) {
		rememberVisibleSize(runtime, runtime.host.parentElement);
	}
	destination.appendChild(runtime.host);

	const parked = destination === runtime.parkingSlot;
	runtime.host.dataset.flowpilotPageRuntimeState = parked
		? "parked"
		: "visible";
	runtime.host.toggleAttribute("inert", parked);
	if (parked) runtime.host.setAttribute("aria-hidden", "true");
	else runtime.host.removeAttribute("aria-hidden");
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
		target: { ...initialTarget },
	};
	runtimes.set(pageId, registration);
	placeRuntime(pageId);

	return {
		updateTarget: (target) => {
			const current = runtimes.get(pageId);
			if (current?.token !== token) return;
			current.target = { ...target };
		},
		unregister: () => {
			const current = runtimes.get(pageId);
			if (current?.token === token) runtimes.delete(pageId);
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

export function getInlineAppPageTarget(
	pageId: string,
): InlineAppPageTarget | undefined {
	const target = runtimes.get(pageId)?.target;
	return target ? { ...target } : undefined;
}
