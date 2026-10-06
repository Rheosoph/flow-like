"use client";

import { useTranslation } from "@flow-like/locales";
import { Ellipsis } from "lucide-react";
import { useRef } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import type {
	FormSessionActions,
	FormSessionState,
	RunEntry,
} from "../contracts";
import type { MenuItem, MenuItemId } from "./bar-actions";
import { blockerText, menuLabel } from "./copy";
import { MENU_CONTENT, MENU_ITEM } from "./menu-style";
import type { PaneEnv } from "./pane-env";

export interface RunMenuProps {
	readonly run: RunEntry;
	readonly items: readonly MenuItem[];
	readonly overlay: FormSessionState["view"]["overlay"];
	readonly env: PaneEnv;
	readonly onCopy: () => void;
}

type MenuAction = (
	run: RunEntry,
	actions: FormSessionActions,
	onCopy: () => void,
) => void;

const MENU_ACTION: Readonly<Record<MenuItemId, MenuAction>> = {
	pin: (run, actions) => actions.pinRun(run.id),
	unpin: (_run, actions) => actions.pinRun(null),
	repeat: (run, actions) => actions.runAgain(run.id),
	use: (run, actions) => actions.useInputs(run.id),
	savePreset: (run, actions) =>
		actions.openOverlay({ id: "presetSave", mode: "save", fromRunId: run.id }),
	copy: (_run, _actions, onCopy) => onCopy(),
	remove: (run, actions) => actions.removeRun(run.id),
};

/** Items whose action moves the cursor itself (to Run, or to the new run's Stop): the menu must not pull it back. */
const OWNS_FOCUS: ReadonlySet<MenuItemId> = new Set<MenuItemId>([
	"use",
	"repeat",
	"remove",
]);

function MenuRow({
	item,
	env,
	onChoose,
}: Readonly<{
	item: MenuItem;
	env: PaneEnv;
	onChoose: (item: MenuItem) => void;
}>) {
	const { t } = useTranslation("interfaces");
	return (
		<>
			{item.separatorBefore ? (
				<DropdownMenuSeparator className="my-1 bg-hairline" />
			) : null}
			<DropdownMenuItem
				disabled={item.disabled}
				onSelect={() => onChoose(item)}
				className={cx(
					MENU_ITEM,
					env.touch ? "min-h-11" : "min-h-8",
					item.id === "remove" &&
						"text-critical focus:bg-critical-bg focus:text-critical",
				)}
			>
				{menuLabel(t, item)}
			</DropdownMenuItem>
			{item.disabled && item.blocker ? (
				<p className="px-2 pb-1.5 text-muted-foreground text-xs/4">
					{blockerText(t, item.blocker)}
				</p>
			) : null}
		</>
	);
}

/** The "…" menu of an ended run (spec M1, M5, S1, S4): pin, repeat, use these inputs, save as a preset, copy, remove. */
export function RunMenu({
	run,
	items,
	overlay,
	env,
	onCopy,
}: Readonly<RunMenuProps>) {
	const { t } = useTranslation("interfaces");
	const { actions } = env;
	const open = overlay?.id === "runMenu" && overlay.runId === run.id;
	const keepOverlay = useRef(false);
	const ownsFocus = useRef(false);
	const label = t("workbench.stage.menu.label", "More actions for run {{n}}", {
		n: run.n,
	});
	const choose = (item: MenuItem) => {
		ownsFocus.current = OWNS_FOCUS.has(item.id);
		keepOverlay.current = item.id === "savePreset";
		MENU_ACTION[item.id](run, actions, onCopy);
	};
	const onOpenChange = (next: boolean) => {
		if (next) actions.openOverlay({ id: "runMenu", runId: run.id });
		else if (keepOverlay.current) keepOverlay.current = false;
		else if (open) actions.closeOverlay();
	};
	return (
		<DropdownMenu modal={false} open={open} onOpenChange={onOpenChange}>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					aria-label={label}
					title={label}
					className={cx(
						"inline-flex items-center justify-center rounded-lg border border-border bg-card text-foreground outline-ring hover:border-border-strong hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-2",
						env.touch ? "size-11" : "size-8",
					)}
				>
					<Ellipsis aria-hidden className="size-3.75" />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="end"
				className={cx(MENU_CONTENT, "min-w-54")}
				onCloseAutoFocus={(event) => {
					if (!ownsFocus.current) return;
					ownsFocus.current = false;
					event.preventDefault();
				}}
			>
				{items.map((item) => (
					<MenuRow key={item.id} item={item} env={env} onChoose={choose} />
				))}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
