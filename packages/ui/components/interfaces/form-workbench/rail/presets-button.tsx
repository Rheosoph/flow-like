"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { Bookmark, ChevronDown } from "lucide-react";
import type { ComponentProps } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { FOCUS_VALUE } from "../contracts";
import { focusProps } from "./focus";

export interface PresetsButtonProps
	extends Omit<ComponentProps<"button">, "children"> {
	/** The active preset's name; null shows "Presets". */
	readonly activeName: string | null;
	/** Inputs the preset sets that now differ from it. */
	readonly edited: number;
	readonly mac: boolean;
	/** Touch screens: 44 px high. */
	readonly touch: boolean;
}

type InterfacesT = TFunction<"interfaces">;

function labelOf(t: InterfacesT, name: string | null, edited: number) {
	if (name === null)
		return t(
			"interfaces:workbench.preset.button.aria.none",
			"No preset. Open presets",
		);
	if (edited === 0)
		return t(
			"interfaces:workbench.preset.button.aria.active",
			"Preset: {{name}}. Open presets",
			{ name },
		);
	return t(
		"interfaces:workbench.preset.button.aria.edited",
		"Preset: {{name}}, {{count}} inputs edited. Open presets",
		{
			name,
			count: edited,
			defaultValue_one:
				"Preset: {{name}}, {{count}} input edited. Open presets",
		},
	);
}

/** S1: the Presets button at the right end of the Inputs | Runs row (beside the name on a narrow box). */
export function PresetsButton({
	activeName,
	edited,
	mac,
	touch,
	className,
	...props
}: Readonly<PresetsButtonProps>) {
	const { t } = useTranslation("interfaces");
	const key = mac ? "⌘P" : "Ctrl+P";
	return (
		<button
			type="button"
			{...focusProps(FOCUS_VALUE.presetButton)}
			aria-label={labelOf(t, activeName, edited)}
			title={t("workbench.preset.button.title", "Presets · {{key}}", { key })}
			aria-keyshortcuts={mac ? "Meta+P" : "Control+P"}
			className={cx(
				"inline-flex min-w-0 max-w-42 flex-none items-center gap-1.5 rounded-md border border-border bg-card text-[12.5px] font-medium hover:border-border-strong hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
				touch ? "h-11 pr-2 pl-3 text-sm" : "h-6 pr-1.5 pl-2",
				className,
			)}
			{...props}
		>
			<Bookmark aria-hidden className="size-3.25 shrink-0 text-ink-2" />
			<span
				className={cx(
					"min-w-0 truncate",
					activeName ? "text-foreground" : "text-ink-2",
				)}
			>
				{activeName ?? t("workbench.preset.button.label", "Presets")}
			</span>
			{edited > 0 ? (
				<span
					aria-hidden
					className="size-1.5 flex-none rounded-full bg-foreground"
				/>
			) : null}
			<ChevronDown
				aria-hidden
				className="size-3.25 shrink-0 text-muted-foreground"
			/>
		</button>
	);
}
