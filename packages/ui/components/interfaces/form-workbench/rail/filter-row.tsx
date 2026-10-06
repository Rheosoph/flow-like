"use client";

import { useTranslation } from "@flow-like/locales";
import { Search } from "lucide-react";
import type { KeyboardEvent } from "react";
import { Kbd } from "../../../settings/devices/primitives/kbd";
import { FilterChip } from "../../../settings/devices/primitives/segmented";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	FOCUS_VALUE,
	type RailFilter,
	type WorkbenchLayout,
} from "../contracts";
import { focusProps } from "./focus";
import type { FilterCounts } from "./rail-model";

export interface FilterRowProps {
	readonly filter: RailFilter;
	readonly counts: FilterCounts;
	readonly layout: WorkbenchLayout;
	readonly setFilter: (filter: Partial<RailFilter>) => void;
	/** ↵ in the box: the first field that matches takes the cursor. */
	readonly focusFirst: () => void;
}

const CHIPS: readonly RailFilter["chip"][] = ["all", "required", "changed"];

/** S5: the filter row of a large form: search, three chips, a "/" chip that says which key reaches it. */
export function FilterRow({
	filter,
	counts,
	layout,
	setFilter,
	focusFirst,
}: Readonly<FilterRowProps>) {
	const { t } = useTranslation("interfaces");
	const touch = layout.touch;
	const label = t("workbench.rail.filter.label", "Filter fields");
	const chipLabels: Readonly<Record<RailFilter["chip"], string>> = {
		all: t("workbench.rail.filter.all", "All"),
		required: t("workbench.rail.filter.required", "Required"),
		changed: t("workbench.rail.filter.changed", "Changed"),
	};
	const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
		if (event.nativeEvent.isComposing || event.keyCode === 229) return;
		if (event.key === "Enter") {
			event.preventDefault();
			focusFirst();
		} else if (event.key === "Escape" && filter.query !== "") {
			event.preventDefault();
			event.stopPropagation();
			setFilter({ query: "" });
		}
	};
	return (
		<div className="flex flex-none flex-col gap-2 pb-3">
			<span className="relative block">
				<Search
					aria-hidden
					className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
				/>
				<input
					type="search"
					{...focusProps(FOCUS_VALUE.filter)}
					value={filter.query}
					onChange={(event) => setFilter({ query: event.target.value })}
					onKeyDown={onKeyDown}
					placeholder={label}
					aria-label={label}
					aria-keyshortcuts={layout.finePointer ? "/" : undefined}
					autoComplete="off"
					className={cx(
						"block w-full rounded-md border border-input bg-card pr-9 pl-7.75 text-foreground placeholder:text-muted-foreground hover:border-border-strong focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring [&::-webkit-search-cancel-button]:appearance-none",
						touch ? "h-11 text-base" : "h-8 text-[13px]",
					)}
				/>
				{layout.finePointer && filter.query === "" ? (
					<Kbd
						aria-hidden
						className="absolute top-1/2 right-2 h-5 -translate-y-1/2 rounded px-1.5 text-xs"
					>
						/
					</Kbd>
				) : null}
			</span>
			<fieldset
				aria-label={t("workbench.rail.filter.group", "Which fields to show")}
				className="m-0 flex min-w-0 flex-wrap gap-1.5 border-0 p-0"
			>
				{CHIPS.map((chip) => (
					<FilterChip
						key={chip}
						pressed={filter.chip === chip}
						onPressedChange={() => setFilter({ chip })}
						count={counts[chip]}
						className={cx("gap-[5px] px-3 text-ui", touch ? "h-11" : "h-7")}
					>
						{chipLabels[chip]}
					</FilterChip>
				))}
			</fieldset>
		</div>
	);
}
