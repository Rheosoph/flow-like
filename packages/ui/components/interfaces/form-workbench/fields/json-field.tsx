"use client";

import { useTranslation } from "@flow-like/locales";
import { useRef } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { textOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { textBox } from "./control-style";
import { useAutoGrow } from "./use-auto-grow";

/** The "JSON" hint at the right of a JSON box's label line. */
export function JsonHint() {
	const { t } = useTranslation("interfaces");
	return (
		<span className="shrink-0 font-mono text-xs/4 text-muted-foreground">
			{t("workbench.field.jsonHint", "JSON")}
		</span>
	);
}

/**
 * Generic, geometry and nested shapes: a mono text box parsed as JSON when the run is pressed. ↵ adds a line, Tab moves
 * on, ⌘↵ runs from here like anywhere else.
 */
export function JsonField(props: ControlProps) {
	const { field, rail, actions, bind, disabled } = props;
	const text = textOf(valueAt(rail.values, field.key));
	const element = useRef<HTMLTextAreaElement>(null);
	useAutoGrow(element, text, bind.touch);
	return (
		<textarea
			ref={element}
			id={bind.ids.control}
			rows={4}
			value={text}
			disabled={disabled}
			spellCheck={false}
			autoComplete="off"
			autoCapitalize="off"
			autoCorrect="off"
			aria-invalid={bind.invalid || undefined}
			aria-describedby={bind.describedBy()}
			{...bind.focus}
			onChange={(event) => actions.setValue(field.key, event.target.value)}
			onBlur={() => actions.blurField(field.key)}
			className={cx(
				textBox({ touch: bind.touch, differs: bind.markers.differs }),
				"min-h-24 font-mono text-[12.5px]/4.5",
			)}
		/>
	);
}
