import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import type { SectionKey } from "./pane-model";
import {
	activeSectionOf,
	askingTurn,
	followsTail,
	sectionScrollTop,
	tailNudge,
} from "./scroll-model";

/** The end of the text a live run is writing; the body keeps it in view while the person has not scrolled up. */
export const TAIL_ATTR = "data-fw-tail";

const useIsoLayoutEffect =
	typeof window === "undefined" ? useEffect : useLayoutEffect;

/**
 * Reading position of one run's body (canvas `onBodyScroll`, `goSec`): which section's link is current,
 * a link that scrolls to its section, and following a live answer unless it was scrolled away. While the
 * run asks, nothing is followed and its question is kept in view.
 */
export function useBodyScroll(
	sections: readonly SectionKey[],
	live: boolean,
	asking = false,
) {
	const bodyRef = useRef<HTMLDivElement>(null);
	const follow = useRef(true);
	const wasAsking = useRef(asking);
	const latest = useRef(sections);
	latest.current = sections;
	const [active, setActive] = useState<SectionKey | null>(sections[0] ?? null);
	const sectionKey = sections.join("|");

	const measure = useCallback(() => {
		const body = bodyRef.current;
		const keys = latest.current;
		if (!body) return;
		if (body.clientHeight === 0) {
			setActive(keys[0] ?? null);
			return;
		}
		const frame = body.getBoundingClientRect();
		const tops = keys.flatMap((key) => {
			const element = body.querySelector(`[data-sec="${key}"]`);
			return element
				? [{ key, top: element.getBoundingClientRect().top - frame.top }]
				: [];
		});
		setActive(
			activeSectionOf({
				tops,
				viewHeight: body.clientHeight,
				scrollTop: body.scrollTop,
				scrollHeight: body.scrollHeight,
			}),
		);
		const tail = body.querySelector(`[${TAIL_ATTR}]`);
		follow.current = followsTail(
			tail ? tail.getBoundingClientRect().top : null,
			frame.bottom,
		);
	}, []);

	const goTo = useCallback((key: SectionKey) => {
		const body = bodyRef.current;
		const element = body?.querySelector(`[data-sec="${key}"]`);
		if (!body || !element) return;
		follow.current = false;
		body.scrollTop = sectionScrollTop(
			body.scrollTop,
			element.getBoundingClientRect().top - body.getBoundingClientRect().top,
		);
		setActive(key);
	}, []);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `sectionKey` stands for the sections
	useEffect(() => measure(), [measure, sectionKey]);

	useIsoLayoutEffect(() => {
		const turn = askingTurn(wasAsking.current, asking);
		wasAsking.current = asking;
		const body = bodyRef.current;
		if (turn === "reveal" && body) body.scrollTop = 0;
		if (turn === "resume") follow.current = true;
	}, [asking]);

	useIsoLayoutEffect(() => {
		const body = bodyRef.current;
		const tail = body?.querySelector(`[${TAIL_ATTR}]`);
		if (!live || asking || !follow.current || !body || !tail) return;
		const delta = tailNudge(
			tail.getBoundingClientRect().bottom,
			body.getBoundingClientRect().bottom,
		);
		if (delta > 0) body.scrollTop += delta;
	});

	return { bodyRef, active, onScroll: measure, goTo };
}
