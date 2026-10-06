import { useTranslation } from "@flow-like/locales";
import { type RefObject, useCallback, useMemo } from "react";
import {
	type DockProps,
	FORM_LIMITS,
	type FormSessionActions,
	type FormSessionState,
	type RunTrigger,
	type ShortWords,
} from "../contracts";
import { shortWordsOf } from "../model/date-text";
import {
	type CopyContext,
	type InterfacesT,
	capButton,
	capSentence,
	listJoinerOf,
	runAgainLabel,
	runLabel,
	runTitle,
} from "./copy";
import {
	dockLineOf,
	knownPerRunNames,
	phoneLineOf,
	quickCapOf,
	runLabelKindOf,
} from "./dock-view";
import { type LineView, lineViewOf } from "./line-view";

export interface DockWords {
	readonly t: InterfacesT;
	readonly words: ShortWords;
	readonly copy: CopyContext;
}

/** `t`, the short value words and the context the dock's texts need (key names, list joining, size limits). */
export function useDockWords(state: FormSessionState): DockWords {
	const { t, i18n } = useTranslation("interfaces");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const { viewer, host } = state.form;
	const words = useMemo(
		() => shortWordsOf(t, viewer.locale),
		[t, viewer.locale],
	);
	const copy = useMemo(
		() => ({
			mac: viewer.mac,
			decimalSign: viewer.decimalSign,
			warnBytes: host.warnFileBytes ?? FORM_LIMITS.warnFileBytes,
			list: listJoinerOf(language),
		}),
		[viewer.mac, viewer.decimalSign, host.warnFileBytes, language],
	);
	return useMemo(() => ({ t, words, copy }), [t, words, copy]);
}

/** Shows every field again: the Inputs tab, the Inputs pane and an empty filter (the ▲▼ buttons need the field in the page). */
export function useReveal(actions: FormSessionActions) {
	return useCallback(() => {
		actions.setRailTab("inputs");
		actions.setPane("inputs");
		actions.setFilter({ query: "", chip: "all" });
	}, [actions]);
}

/** The status line as drawn: the dock line, trimmed to what the narrow layout shows when `phone`. */
export function useStatusLine(
	state: FormSessionState,
	actions: FormSessionActions,
	kit: DockWords,
	phone: boolean,
) {
	const line = useMemo(() => dockLineOf(state, kit.words), [state, kit.words]);
	const view = useMemo((): LineView | null => {
		const shown = phone ? phoneLineOf(state, line) : line;
		if (!shown) return null;
		return lineViewOf(shown, {
			t: kit.t,
			copy: kit.copy,
			words: kit.words,
			actions,
			state,
		});
	}, [line, phone, state, actions, kit]);
	return { line, view };
}

const TRIGGER_OF: Readonly<Record<DockProps["variant"], RunTrigger>> = {
	rail: "button",
	phone: "phone",
	hero: "hero",
	strip: "strip",
};

export interface RunControl {
	readonly label: string;
	readonly title: string;
	readonly capped: boolean;
	readonly onRun: () => void;
}

/** The Run button's words and press: its label, title, cap state and the trigger the reducer is told about. */
export function useRunControl(
	state: FormSessionState,
	actions: FormSessionActions,
	variant: DockProps["variant"],
	kit: DockWords,
): RunControl {
	const { t, copy } = kit;
	const cap = quickCapOf(state);
	const kind = runLabelKindOf(state, variant);
	const own =
		state.form.submitLabel ??
		(kind === "runAgain" ? runAgainLabel(t) : runLabel(t));
	const anyPerRun = knownPerRunNames(state).length > 0;
	const from = TRIGGER_OF[variant];
	const onRun = useCallback(() => actions.run({ from }), [actions, from]);
	return {
		label: cap && variant !== "phone" ? capButton(t) : own,
		title: cap
			? capSentence(t, cap.running)
			: runTitle(t, own, copy.mac, anyPerRun),
		capped: cap !== null,
		onRun,
	};
}

/** Focus helpers around the Run button: Stop hands the cursor to Run; a keyboard-pressed line button that disappears does too. */
export function useRunFocus(runRef: RefObject<HTMLButtonElement | null>) {
	const focusRun = useCallback(() => runRef.current?.focus(), [runRef]);
	const afterAction = useCallback(
		(viaKeyboard: boolean) => {
			if (!viaKeyboard) return;
			requestAnimationFrame(() => {
				const active = runRef.current?.ownerDocument.activeElement;
				if (!active || active === active.ownerDocument.body)
					runRef.current?.focus();
			});
		},
		[runRef],
	);
	return { focusRun, afterAction };
}
