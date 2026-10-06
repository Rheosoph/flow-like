/*
 * Reading position of a run's body: which section the person is in (a section counts once its top
 * passes 40 % of the view) and whether a live answer is followed. Canvas `onBodyScroll`. Pure.
 */
import type { SectionKey } from "./pane-model";

export interface SectionTop {
	readonly key: SectionKey;
	/** Distance of the section's top from the body's top edge (negative: scrolled past). */
	readonly top: number;
}

export interface ScrollProbe {
	readonly tops: readonly SectionTop[];
	readonly viewHeight: number;
	readonly scrollTop: number;
	readonly scrollHeight: number;
}

/** A section is "being read" once its top is above this share of the view (never closer than 48 px). */
export const READ_LINE = 0.4;
const MIN_READ_LINE = 48;
const END_SLACK = 24;

/**
 * The section whose link is current: the first one while the body is at the top, else the last one past the
 * read line; at the very end of a scrolled body, the last one.
 */
export function activeSectionOf(probe: ScrollProbe): SectionKey | null {
	const first = probe.tops[0];
	if (!first) return null;
	if (probe.scrollTop <= 0) return first.key;
	const line = Math.max(MIN_READ_LINE, probe.viewHeight * READ_LINE);
	let key = first.key;
	for (const section of probe.tops) if (section.top <= line) key = section.key;
	const atEnd =
		probe.scrollTop + probe.viewHeight >= probe.scrollHeight - END_SLACK;
	const last = probe.tops[probe.tops.length - 1];
	return atEnd && probe.scrollTop > 0 && last ? last.key : key;
}

/**
 * A live answer is followed while its end is in view or below it; scrolling up past it stops the
 * following, scrolling back down resumes it. No end (nothing to follow yet) counts as following.
 */
export const followsTail = (tailTop: number | null, viewBottom: number) =>
	tailTop === null || tailTop <= viewBottom;

/** The canvas keeps the streaming caret this far above the bottom edge of the body (its `afterRender`). */
export const CARET_GAP = 88;

/** How far to scroll so the end of the live text ends `gap` px above the bottom edge. */
export const tailNudge = (
	tailBottom: number,
	viewBottom: number,
	gap = CARET_GAP,
) => Math.max(0, tailBottom - (viewBottom - gap));

/**
 * A run that starts asking stops being followed and shows its question, the first thing in its body;
 * once answered it is followed again, since the form moved the reader and the reader did not scroll.
 */
export const askingTurn = (
	wasAsking: boolean,
	asking: boolean,
): "reveal" | "resume" | null => {
	if (asking === wasAsking) return null;
	return asking ? "reveal" : "resume";
};

/** Where a section link scrolls to: the section's top 20 px below the body's top edge. */
export const sectionScrollTop = (
	scrollTop: number,
	sectionTop: number,
	gap = 20,
) => Math.max(0, scrollTop + sectionTop - gap);
