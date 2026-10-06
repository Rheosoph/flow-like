"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "../../../settings/devices/primitives/tone";
import type { FormModel, FormSessionState } from "../contracts";
import { type PaneEnv, paneIdOf, tabIdOf } from "./pane-env";
import type { PaneView } from "./pane-model";
import { RunBar } from "./run-bar";
import { RunBody } from "./run-body";
import { SectionLinks } from "./section-links";
import { useBodyScroll } from "./use-body-scroll";

export interface RunPaneProps {
	readonly pane: PaneView;
	readonly env: PaneEnv;
	readonly overlay: FormSessionState["view"]["overlay"];
	readonly form: Pick<FormModel, "name" | "description" | "routes">;
	/** The run on the stage: its tab controls this pane. */
	readonly selected: boolean;
	/** Two panes: the lead names its run, the pinned one shows the pin; the second has an edge line. */
	readonly comparing: boolean;
	readonly pinned: boolean;
	readonly edge: boolean;
}

const WORKING = new Set(["starting", "running", "streaming", "asking"]);

/** One run: its bar, change chips, section links and scrolling body. Two of these sit side by side in Compare. */
export function RunPane({
	pane,
	env,
	overlay,
	form,
	selected,
	comparing,
	pinned,
	edge,
}: Readonly<RunPaneProps>) {
	const { t } = useTranslation("interfaces");
	const { run } = pane;
	const live = WORKING.has(run.status);
	const scroll = useBodyScroll(pane.sections, live, run.status === "asking");
	return (
		<section
			id={paneIdOf(env.uid, run.id)}
			role={selected ? "tabpanel" : undefined}
			aria-labelledby={selected ? tabIdOf(env.uid, run.id) : undefined}
			aria-label={
				selected
					? undefined
					: t("workbench.stage.pane.label", "Run {{n}}", { n: run.n })
			}
			data-fw-pane={run.id}
			className={cx(
				"flex min-h-0 min-w-0 flex-1 flex-col",
				edge && "border-hairline border-l",
			)}
		>
			<RunBar
				pane={pane}
				env={env}
				overlay={overlay}
				named={comparing}
				pinned={pinned}
				onGoInputs={() => scroll.goTo("inputs")}
			/>
			<SectionLinks
				runN={run.n}
				sections={pane.showLinks ? pane.sections : []}
				active={scroll.active}
				fileCount={run.output?.attachments.length ?? 0}
				env={env}
				onGo={scroll.goTo}
			/>
			<RunBody
				pane={pane}
				env={env}
				form={form}
				bodyRef={scroll.bodyRef}
				onScroll={scroll.onScroll}
			/>
		</section>
	);
}
