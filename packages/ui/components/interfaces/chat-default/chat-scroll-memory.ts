/** Where a reader left a transcript: the first row in view and its top's distance from the viewport's top. */
export interface ChatScrollMark {
	readonly rowId: string;
	readonly offset: number;
}

const ROW_ATTRIBUTE = "data-chat-message";
const MAX_REMEMBERED = 64;
/** How long a restored row is held in place while rows above it mount and late content settles. */
const HOLD_MS = 1200;
const RELEASE_EVENTS = ["wheel", "touchstart", "pointerdown", "keydown"];

const marks = new Map<string, ChatScrollMark>();

/** Remembers where the reader left `sessionId`; `null` means at the end, where a reopened chat starts anyway. */
export function rememberChatScroll(
	sessionId: string,
	mark: ChatScrollMark | null,
): void {
	marks.delete(sessionId);
	if (!mark) return;
	marks.set(sessionId, mark);
	if (marks.size <= MAX_REMEMBERED) return;
	const oldest = marks.keys().next().value;
	if (oldest !== undefined) marks.delete(oldest);
}

export function recallChatScroll(
	sessionId: string | undefined,
): ChatScrollMark | undefined {
	return sessionId ? marks.get(sessionId) : undefined;
}

function rows(container: HTMLElement): HTMLElement[] {
	const found: HTMLElement[] = [];
	for (const child of container.children) {
		if (child.hasAttribute(ROW_ATTRIBUTE)) found.push(child as HTMLElement);
	}
	return found;
}

function findRow(container: HTMLElement, rowId: string): HTMLElement | null {
	for (const row of rows(container)) {
		if (row.getAttribute(ROW_ATTRIBUTE) === rowId) return row;
	}
	return null;
}

/** The first row reaching below the container's top edge, or `null` while the container shows none. */
export function readChatScrollMark(
	container: HTMLElement,
): ChatScrollMark | null {
	if (container.clientHeight === 0) return null;
	const top = container.getBoundingClientRect().top;
	const list = rows(container);
	let low = 0;
	let high = list.length - 1;
	let first: HTMLElement | null = null;
	while (low <= high) {
		const middle = (low + high) >> 1;
		const row = list[middle];
		if (row.getBoundingClientRect().bottom > top) {
			first = row;
			high = middle - 1;
		} else {
			low = middle + 1;
		}
	}
	const rowId = first?.getAttribute(ROW_ATTRIBUTE);
	if (!first || !rowId) return null;
	return { rowId, offset: first.getBoundingClientRect().top - top };
}

/**
 * Scrolls the marked row back to its offset and keeps it there while rows above it mount and late
 * content settles. The reader's first wheel, touch, click or key hands the scroll back. Returns the
 * release, or `null` when the row is not in the transcript.
 */
export function holdChatScrollMark(
	container: HTMLElement,
	mark: ChatScrollMark,
): (() => void) | null {
	let row = findRow(container, mark.rowId);
	if (!row) return null;
	let frame = 0;
	const deadline = performance.now() + HOLD_MS;

	const align = () => {
		if (!row?.isConnected) row = findRow(container, mark.rowId);
		if (!row) return false;
		const drift =
			row.getBoundingClientRect().top -
			container.getBoundingClientRect().top -
			mark.offset;
		if (Math.abs(drift) >= 1) container.scrollTop += drift;
		return true;
	};
	const release = () => {
		cancelAnimationFrame(frame);
		for (const type of RELEASE_EVENTS) {
			container.removeEventListener(type, release);
		}
	};
	const tick = () => {
		if (performance.now() > deadline || !align()) {
			release();
			return;
		}
		frame = requestAnimationFrame(tick);
	};

	for (const type of RELEASE_EVENTS) {
		container.addEventListener(type, release, { passive: true });
	}
	align();
	frame = requestAnimationFrame(tick);
	return release;
}
