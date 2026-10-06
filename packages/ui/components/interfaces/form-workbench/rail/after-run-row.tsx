"use client";

import { useTranslation } from "@flow-like/locales";
import { perRunCount } from "./rail-model";
import type { RailPartProps } from "./rail-model";

/**
 * M1 on a phone: the after-run line becomes a 44 px row at the end of the Inputs pane; "Change" opens
 * the Per run sheet (the dock owns it).
 */
export function AfterRunRow({
	state,
	actions,
}: Readonly<Omit<RailPartProps, "layout">>) {
	const { t } = useTranslation("interfaces");
	const count = perRunCount(state);
	return (
		<div className="mx-auto flex min-h-11 w-full max-w-160 items-center gap-1.5 px-4 pb-7 text-sm/5">
			<span className="text-muted-foreground">
				{count === 0
					? t("workbench.rail.afterRun.none", "No input is per run")
					: t(
							"workbench.rail.afterRun.summary",
							"{{count}} inputs are per run",
							{
								count,
								defaultValue_one: "{{count}} input is per run",
							},
						)}
			</span>
			<span aria-hidden className="text-muted-foreground">
				·
			</span>
			<button
				type="button"
				aria-label={t(
					"workbench.rail.afterRun.changeAria",
					"Change which inputs are per run",
				)}
				onClick={() => actions.openOverlay({ id: "afterRun", focusName: null })}
				className="min-h-11 rounded-md px-1 font-medium text-ink-2 hover:underline focus-visible:outline-2 focus-visible:outline-ring"
			>
				{t("workbench.rail.afterRun.change", "Change")}
			</button>
		</div>
	);
}
