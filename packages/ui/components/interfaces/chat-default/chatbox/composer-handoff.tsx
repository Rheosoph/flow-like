"use client";

import {
	type ReactNode,
	type RefObject,
	createContext,
	useContext,
	useLayoutEffect,
	useMemo,
	useRef,
} from "react";

export interface ComposerDraft {
	readonly text: string;
	readonly files: File[];
	readonly focused: boolean;
}

interface ParkedDraft extends ComposerDraft {
	readonly scope?: string;
}

interface ComposerHandoff {
	readonly scope?: string;
	readonly slot: { current: ParkedDraft | null };
}

const ComposerHandoffContext = createContext<ComposerHandoff | null>(null);

/**
 * A chat swaps its composer whenever it moves between loading, the welcome and the transcript.
 * Inside this provider the leaving composer parks its draft and focus, and the next one mounted
 * for the same `scope` (conversation) takes them, so the swap neither clears what the user typed
 * nor drops the soft keyboard.
 */
export function ComposerHandoffProvider({
	scope,
	children,
}: Readonly<{ scope?: string; children: ReactNode }>) {
	const slot = useRef<ParkedDraft | null>(null);
	const value = useMemo(() => ({ scope, slot }), [scope]);
	return (
		<ComposerHandoffContext.Provider value={value}>
			{children}
		</ComposerHandoffContext.Provider>
	);
}

/**
 * `read` must return the draft the user sees right now, even when the composer unmounts in the
 * same batch that cleared it.
 */
export function useComposerHandoff(
	field: RefObject<HTMLTextAreaElement | null>,
	read: () => Omit<ComposerDraft, "focused">,
	restore: (draft: ComposerDraft) => void,
) {
	const handoff = useContext(ComposerHandoffContext);
	const latest = useRef({ read, restore });
	latest.current = { read, restore };

	useLayoutEffect(() => {
		if (!handoff) return;
		const { scope, slot } = handoff;
		const element = field.current;
		const parked = slot.current;
		slot.current = null;
		if (parked && parked.scope === scope) {
			latest.current.restore(parked);
			if (parked.focused) element?.focus({ preventScroll: true });
		}
		// Layout cleanup runs before React detaches the DOM, so focus is still readable here.
		return () => {
			const focused =
				!!element && element.ownerDocument.activeElement === element;
			const draft = latest.current.read();
			slot.current =
				draft.text || draft.files.length > 0 || focused
					? { ...draft, focused, scope }
					: null;
		};
	}, [handoff, field]);
}
