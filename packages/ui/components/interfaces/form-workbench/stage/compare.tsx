"use client";

import { useTranslation } from "@flow-like/locales";
import { DvButton } from "../../../settings/devices/primitives/dv-button";
import { cx } from "../../../settings/devices/primitives/tone";
import type { FieldChange, ShortWords } from "../contracts";
import type { CompareTitle, CompareView } from "./compare-model";
import type { InterfacesT } from "./copy";
import type { PaneEnv } from "./pane-env";

/** The numbers of the pinned run and of the run on the stage. */
const numbersOf = (view: CompareView) => ({
	a: view.pinned?.n ?? 0,
	b: view.selected?.n ?? 0,
});

type TitleWords = (
	t: InterfacesT,
	view: CompareView,
	n: { a: number; b: number },
) => string;

const TITLE: Readonly<Record<CompareTitle["kind"], TitleWords>> = {
	same: (t) => t("interfaces:workbench.stage.compare.same", "Same inputs"),
	differ: (t, view) =>
		t("interfaces:workbench.stage.compare.differ", "{{count}} inputs differ", {
			count: view.rows.length,
			defaultValue_one: "{{count}} input differs",
		}),
	beside: (t, _view, n) =>
		t(
			"interfaces:workbench.stage.compare.beside",
			"Run {{a}} beside run {{b}}",
			n,
		),
	pinned: (t, _view, n) =>
		t("interfaces:workbench.stage.compare.pinned", "Run {{n}} is pinned", {
			n: n.a,
		}),
};

const titleText = (t: InterfacesT, view: CompareView) =>
	TITLE[view.title.kind](t, view, numbersOf(view));

const widenText = (t: InterfacesT, view: CompareView) =>
	view.samePinned
		? t(
				"interfaces:workbench.stage.compare.widen",
				"Widen the window to see another run beside it.",
			)
		: t(
				"interfaces:workbench.stage.compare.widenBeside",
				"Widen the window to see it beside run {{n}}.",
				{ n: numbersOf(view).b },
			);

const sameText = (t: InterfacesT, view: CompareView) =>
	t(
		"interfaces:workbench.stage.compare.sameSub",
		"Run {{a}} and run {{b}} were given the same inputs.",
		numbersOf(view),
	);

/** The grey line beside the title: why there is no second pane, or that two runs got the same inputs. */
function subText(t: InterfacesT, view: CompareView) {
	if (view.mode === "note") return widenText(t, view);
	return view.title.kind === "same" ? sameText(t, view) : "";
}

function Cell({
	text,
	words,
	className,
}: Readonly<{ text: string; words: ShortWords; className: string }>) {
	const empty = text === words.empty || text === words.none;
	return (
		<td
			className={cx(
				"border-0 border-hairline border-t px-3 py-1.5 wrap-anywhere",
				empty && "text-muted-foreground",
				className,
			)}
		>
			{text}
		</td>
	);
}

function DiffRow({
	row,
	words,
}: Readonly<{ row: FieldChange; words: ShortWords }>) {
	return (
		<tr>
			<th
				scope="row"
				className="overflow-hidden text-ellipsis whitespace-nowrap border-0 border-hairline border-t px-3 py-1.5 text-left font-medium"
			>
				{row.label}
			</th>
			<Cell text={row.from} words={words} className="" />
			<Cell
				text={row.to}
				words={words}
				className="border-hairline border-l pl-5.75"
			/>
		</tr>
	);
}

function DiffTable({
	view,
	words,
	narrow,
}: Readonly<{ view: CompareView; words: ShortWords; narrow: boolean }>) {
	const { t } = useTranslation("interfaces");
	const head =
		"border-0 bg-surface-sunken px-3 py-1.5 text-left font-medium text-muted-foreground";
	return (
		<div className="overflow-hidden rounded-lg border border-border bg-card">
			<table className="m-0 w-full table-fixed border-collapse text-[13px]/[18px]">
				<thead>
					<tr>
						<th
							scope="col"
							className={head}
							style={{ width: narrow ? 132 : 168 }}
						>
							{t("workbench.stage.compare.input", "Input")}
						</th>
						<th scope="col" className={head}>
							{t("workbench.stage.compare.pinnedHead", "Run {{n}} · pinned", {
								n: view.pinned?.n ?? 0,
							})}
						</th>
						<th
							scope="col"
							className={cx(head, "w-1/2 border-hairline border-l pl-5.75")}
						>
							{t("workbench.stage.compare.runHead", "Run {{n}}", {
								n: view.selected?.n ?? 0,
							})}
						</th>
					</tr>
				</thead>
				<tbody>
					{view.rows.map((row) => (
						<DiffRow key={row.name} row={row} words={words} />
					))}
				</tbody>
			</table>
		</div>
	);
}

/**
 * The strip of Compare: what differs between the pinned run and the one on the stage, with "Unpin run N". On a
 * box too narrow for two panes it is one line naming the pinned run (spec fix-report "Compare").
 */
export function CompareHeader({
	view,
	words,
	env,
	stageWidth,
}: Readonly<{
	view: CompareView;
	words: ShortWords;
	env: PaneEnv;
	stageWidth: number;
}>) {
	const { t } = useTranslation("interfaces");
	const pinned = view.pinned;
	if (!pinned) return null;
	const sub = subText(t, view);
	return (
		<div
			data-fw-compare={view.mode}
			className="flex shrink-0 flex-col gap-2 border-hairline border-b px-6 pt-3 pb-3.5"
		>
			<div className="flex min-h-7 items-center gap-2.5">
				<h2 className="m-0 font-semibold text-[13px]/[18px] tracking-normal">
					{titleText(t, view)}
				</h2>
				<span className="min-w-0 flex-1 text-[12.5px] text-muted-foreground">
					{sub}
				</span>
				<DvButton
					size="sm"
					className={cx("shrink-0 whitespace-nowrap", env.touch && "h-11 px-3")}
					onClick={() => env.actions.pinRun(null)}
				>
					{t("workbench.stage.compare.unpin", "Unpin run {{n}}", {
						n: pinned.n,
					})}
				</DvButton>
			</div>
			{view.rows.length > 0 ? (
				<DiffTable view={view} words={words} narrow={stageWidth < 760} />
			) : null}
		</div>
	);
}

/** The second pane while the pinned run is the one on the stage: nothing to put beside it yet. */
export function ComparePlaceholder({ pinned }: Readonly<{ pinned: number }>) {
	const { t } = useTranslation("interfaces");
	return (
		<section
			aria-label={t("workbench.stage.compare.secondRun", "Second run")}
			className="flex min-h-0 min-w-0 flex-1 items-center justify-center border-hairline border-l p-8"
		>
			<p className="m-0 max-w-88 text-balance text-center text-muted-foreground text-sm/[21px]">
				{t(
					"workbench.stage.compare.hint",
					"Run {{n}} is pinned. Pick another run, or change an input and press Run, to see the two side by side.",
					{ n: pinned },
				)}
			</p>
		</section>
	);
}
