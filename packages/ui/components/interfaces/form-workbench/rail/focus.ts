import { FOCUS_ATTR, FOCUS_VALUE } from "../contracts";

/** The attribute the shell's focus executor finds an element by (`data-fw-focus`). */
export const focusProps = (value: string) => ({ [FOCUS_ATTR]: value });

/** Selector of every focusable field control inside the rail, in document order. */
export const FIELD_FOCUS_SELECTOR = `[${FOCUS_ATTR}^="${FOCUS_VALUE.fieldPrefix}"]`;
