"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useCallback, useEffect, useMemo, useRef } from "react";
import {
	Popover,
	PopoverAnchor,
	PopoverContent,
	PopoverTrigger,
} from "../../../ui/popover";
import { activePresetOf } from "../model/markers";
import { presetEdits } from "../model/presets";
import {
	type FocusSpot,
	focusOnPage,
	focusSpotOf,
	returnFocusTo,
	useIsoLayoutEffect,
} from "../shell/focus";
import { PresetsButton } from "./presets-button";
import { PresetsPanel } from "./presets-panel";
import type { RailPartProps } from "./rail-model";
import { RailSheet } from "./rail-sheet";

const CONTENT =
	"flex max-h-100 w-75 max-w-[calc(100vw-16px)] flex-col rounded-md border border-border-strong bg-popover p-1 text-popover-foreground shadow-floating backdrop-blur-none data-[state=closed]:animate-none data-[state=open]:animate-none";

/** Where focus lands when the menu opens: the active preset, else the first row, else the first action. */
const FIRST_FOCUS = [
	'[aria-current="true"]',
	"[data-preset-id]",
	"[data-preset-item]",
] as const;

function firstItem(root: HTMLElement | null) {
	if (!root) return null;
	for (const selector of FIRST_FOCUS) {
		const found = root.querySelector<HTMLElement>(selector);
		if (found) return found;
	}
	return null;
}

/**
 * Spec §4, Esc: "focus returns". The element that had focus when the menu opened (a field for ⌘P, the
 * Presets button for a click) gets it back when the menu closes and focus fell to the page. Focus an action
 * moved on (a field after a preset is applied) and a click elsewhere stay as they are.
 */
function useReturnFocus() {
	const opener = useRef<FocusSpot | null>(null);
	const outside = useRef(false);
	return useMemo(
		() => ({
			remember() {
				opener.current = focusSpotOf(document);
				outside.current = false;
			},
			clickedOutside() {
				outside.current = true;
			},
			/**
			 * A panel action is about to run (`release`): focus goes back first, so what the action opens
			 * (the save dialog) returns there when it closes, and what it focuses takes over from there.
			 */
			returnNow() {
				returnFocusTo(opener.current);
			},
			/** True when it put focus back. */
			giveBack() {
				const from = opener.current;
				const clicked = outside.current;
				opener.current = null;
				outside.current = false;
				return !clicked && focusOnPage(document) && returnFocusTo(from);
			},
		}),
		[],
	);
}

type ReturnFocus = ReturnType<typeof useReturnFocus>;

/** The sheet is a Radix dialog without a trigger: remember on open, give back on close. */
function useSheetFocus(open: boolean, focus: ReturnFocus) {
	const wasOpen = useRef(false);
	useIsoLayoutEffect(() => {
		if (open && !wasOpen.current) focus.remember();
	}, [open, focus]);
	useEffect(() => {
		if (wasOpen.current && !open) focus.giveBack();
		wasOpen.current = open;
	}, [open, focus]);
}

export interface PresetsMenuProps extends RailPartProps {
	/** The Presets button shows (S1 visibility); without it ⌘P still opens the menu at the same place. */
	readonly visible: boolean;
}

/** The button is the popover's trigger; without a button an empty anchor marks the same place. */
function MenuAnchor({
	visible,
	children,
}: Readonly<{ visible: boolean; children: ReactNode }>) {
	if (visible) return <PopoverTrigger asChild>{children}</PopoverTrigger>;
	return (
		<PopoverAnchor asChild>
			<span aria-hidden className="-ml-1 block size-0 flex-none" />
		</PopoverAnchor>
	);
}

/** The Presets button as the head draws it; on a touch screen it opens the sheet itself. */
function menuButton({ state, actions, layout }: RailPartProps) {
	const active = activePresetOf(state);
	const edited = presetEdits(state.form.fields, state.rail.values, active);
	const openMenu = () => actions.openOverlay({ id: "presets" });
	return (
		<PresetsButton
			activeName={active?.name ?? null}
			edited={edited}
			mac={state.form.viewer.mac}
			touch={layout.touch}
			onClick={layout.touch ? openMenu : undefined}
		/>
	);
}

interface SheetProps extends PresetsMenuProps {
	readonly button: ReactNode;
	readonly title: string;
}

/** A touch screen's menu: a bottom sheet over the interface. */
function PresetsSheet(props: Readonly<SheetProps>) {
	const { state, actions, layout, visible, button, title } = props;
	const open = state.view.overlay?.id === "presets";
	const focus = useReturnFocus();
	useSheetFocus(open, focus);
	return (
		<>
			{visible ? button : null}
			<RailSheet
				open={open}
				onClose={() => actions.closeOverlay()}
				title={title}
				variant="bottom"
			>
				<div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-2">
					<PresetsPanel
						state={state}
						actions={actions}
						layout={layout}
						release={focus.returnNow}
					/>
				</div>
			</RailSheet>
		</>
	);
}

/**
 * The Presets button and its menu. `overlay.id === "presets"` is the open state, so ⌘P (the shell)
 * and the button open the same thing. A popover under the button; a bottom sheet on a touch screen.
 */
export function PresetsMenu(props: Readonly<PresetsMenuProps>) {
	const { state, actions, layout, visible } = props;
	const { t } = useTranslation("interfaces");
	const focus = useReturnFocus();
	const content = useRef<HTMLDivElement | null>(null);
	const button = menuButton(props);
	const title = t("workbench.preset.menu.title", "Presets");
	const changeOpen = (next: boolean) =>
		next ? actions.openOverlay({ id: "presets" }) : actions.closeOverlay();
	const takeFocus = useCallback(
		(event: Event) => {
			event.preventDefault();
			focus.remember();
			firstItem(content.current)?.focus();
		},
		[focus],
	);
	const giveFocusBack = useCallback(
		(event: Event) => {
			if (focus.giveBack() || !focusOnPage(document)) event.preventDefault();
		},
		[focus],
	);
	if (layout.touch)
		return <PresetsSheet {...props} button={button} title={title} />;
	return (
		<Popover
			open={state.view.overlay?.id === "presets"}
			onOpenChange={changeOpen}
		>
			<MenuAnchor visible={visible}>{button}</MenuAnchor>
			<PopoverContent
				ref={content}
				align="end"
				sideOffset={4}
				aria-label={title}
				className={CONTENT}
				onOpenAutoFocus={takeFocus}
				onCloseAutoFocus={giveFocusBack}
				onInteractOutside={focus.clickedOutside}
			>
				<PresetsPanel
					state={state}
					actions={actions}
					layout={layout}
					release={focus.returnNow}
				/>
			</PopoverContent>
		</Popover>
	);
}
