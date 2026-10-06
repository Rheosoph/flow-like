"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "../../../settings/devices/primitives/tone";
import type { ControlProps } from "./bind";

/**
 * A data or value type this client does not know: no control, a plain note. The note takes the focus attribute so a
 * required one can still be the first problem the form points at.
 */
export function UnsupportedField(props: ControlProps) {
	const { t } = useTranslation("interfaces");
	const { bind } = props;
	return (
		<p
			id={bind.ids.control}
			tabIndex={-1}
			aria-describedby={bind.describedBy()}
			{...bind.focus}
			className={cx(
				"m-0 rounded-lg border border-dashed border-border bg-surface-sunken px-2.75 py-2 text-[13px]/4.5 text-muted-foreground outline-none",
			)}
		>
			{t(
				"workbench.field.unsupported",
				"This kind of input can't be filled in here.",
			)}
		</p>
	);
}
