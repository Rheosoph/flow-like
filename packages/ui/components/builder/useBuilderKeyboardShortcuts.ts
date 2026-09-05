import { type RefObject, useEffect } from "react";
import {
	DEFAULT_SHORTCUTS,
	createShortcutManager,
	isEditableKeyboardTarget,
} from "../../lib/builder/KeyboardShortcuts";
import type { BuilderContextType } from "./BuilderContext";

const ACTIONS = new Set([
	"copy",
	"cut",
	"paste",
	"duplicate",
	"delete",
	"undo",
	"redo",
]);
const BUILDER_SHORTCUTS = DEFAULT_SHORTCUTS.filter((shortcut) =>
	ACTIONS.has(shortcut.action),
);

type BuilderKeyboardActions = Pick<
	BuilderContextType,
	| "selection"
	| "copy"
	| "cut"
	| "paste"
	| "duplicate"
	| "deleteComponents"
	| "undo"
	| "redo"
>;

function ownsTarget(root: HTMLElement, target: EventTarget | null): boolean {
	const element = target as Element | null;
	const builderRoot = element?.closest?.("[data-builder-root]");
	if (builderRoot) return builderRoot === root;
	const owner = element
		?.closest?.("[data-builder-owner]")
		?.getAttribute("data-builder-owner");
	return Boolean(owner && owner === root.getAttribute("data-builder-root"));
}

export function useBuilderKeyboardShortcuts(
	rootRef: RefObject<HTMLDivElement | null>,
	enabled: boolean,
	actions: BuilderKeyboardActions,
) {
	useEffect(() => {
		const root = rootRef.current;
		if (!enabled || !root) return;

		const manager = createShortcutManager(
			(action, event) => {
				event.stopPropagation();
				if (event.repeat) return;

				switch (action) {
					case "delete": {
						const ids = actions.selection.componentIds.filter(
							(id) => id !== "root",
						);
						if (ids.length > 0) actions.deleteComponents(ids);
						break;
					}
					case "copy":
						actions.copy();
						break;
					case "cut":
						actions.cut();
						break;
					case "paste":
						actions.paste();
						break;
					case "duplicate":
						actions.duplicate();
						break;
					case "undo":
						actions.undo();
						break;
					case "redo":
						actions.redo();
						break;
				}
			},
			BUILDER_SHORTCUTS,
			(event) => ownsTarget(root, event.target),
		);

		// Canvas elements often have no focusable DOM node. Move focus into the
		// builder after selection so their next keyboard event reaches this editor.
		const focusEditor = (event: MouseEvent) => {
			const target = event.target as HTMLElement | null;
			if (!target || !ownsTarget(root, target) || !root.contains(target)) {
				return;
			}
			if (target.closest("[data-builder-component]")) {
				root.focus({ preventScroll: true });
				return;
			}
			if (isEditableKeyboardTarget(target)) return;
			const focusable = target.closest(
				'a[href], button, input, textarea, select, [tabindex], [contenteditable]:not([contenteditable="false"])',
			);
			if (!focusable || focusable === root) {
				root.focus({ preventScroll: true });
			}
		};

		manager.bind();
		root.ownerDocument.addEventListener("click", focusEditor, true);
		return () => {
			manager.unbind();
			root.ownerDocument.removeEventListener("click", focusEditor, true);
		};
	}, [actions, enabled, rootRef]);
}
