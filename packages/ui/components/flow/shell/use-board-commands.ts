"use client";

import type { LucideIcon } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import { useSpotlightStore } from "../../../state/spotlight-state";

/**
 * Where a command is visible, besides the palette.
 *
 * Required, and deliberately so: a command with no declared surface is reachable
 * only by search, which is how Templates and Auto Layout silently lost their
 * buttons when the dock was replaced. `"palette"` is still a valid answer — it
 * just has to be an answer.
 */
export type IBoardCommandSurface =
	/** Activity rail, upper group — regions of the shell. */
	| "rail"
	/** Activity rail, lower group — navigation and assistants. */
	| "rail-bottom"
	/** Editor tab strip — actions on the open document. */
	| "editor"
	/** Status bar, wired by the shell itself. */
	| "status"
	/** Reachable by chord and search only. */
	| "palette";

export interface IBoardCommand {
	id: string;
	title: string;
	icon?: LucideIcon;
	/** Chord in `mod+shift+k` form. `mod` is ⌘ on Apple platforms, Ctrl elsewhere. */
	shortcut?: string;
	keywords?: string[];
	description?: string;
	surface: IBoardCommandSurface;
	run: () => void;
	/** Commands are hidden from the palette while this is false, and their chord is inert. */
	when?: boolean;
}

/** The commands a given surface renders, in registry order. */
export function commandsFor(
	commands: readonly IBoardCommand[],
	surface: IBoardCommandSurface,
): IBoardCommand[] {
	return commands.filter(
		(command) => command.surface === surface && command.when !== false,
	);
}

const SPOTLIGHT_SOURCE = "flow-board";
const SPOTLIGHT_GROUP = "board";

const isApple = () =>
	typeof navigator !== "undefined" &&
	/Mac|iPod|iPhone|iPad/.test(navigator.platform);

/** `mod+shift+p` → `⌘⇧P`, for the tooltip and the palette row. */
export function formatShortcut(shortcut: string): string {
	const apple = isApple();
	return shortcut
		.split("+")
		.map((part) => {
			switch (part) {
				case "mod":
					return apple ? "⌘" : "Ctrl";
				case "shift":
					return "⇧";
				case "alt":
					return apple ? "⌥" : "Alt";
				default:
					return part.length === 1 ? part.toUpperCase() : part.toUpperCase();
			}
		})
		.join(apple ? "" : "+");
}

function matches(event: KeyboardEvent, shortcut: string): boolean {
	const parts = shortcut.toLowerCase().split("+");
	const key = parts[parts.length - 1];
	const wantMod = parts.includes("mod");
	const wantShift = parts.includes("shift");
	const wantAlt = parts.includes("alt");
	const mod = isApple() ? event.metaKey : event.ctrlKey;
	if (wantMod !== mod) return false;
	if (wantShift !== event.shiftKey) return false;
	if (wantAlt !== event.altKey) return false;
	return event.key.toLowerCase() === key;
}

/** Typing anywhere editable — including the Monaco textarea — never triggers a chord. */
function isEditableTarget(target: EventTarget | null): boolean {
	const element = target as HTMLElement | null;
	if (!element || typeof element.closest !== "function") return false;
	return Boolean(
		element.closest(
			"input, textarea, select, [contenteditable='true'], .monaco-editor",
		),
	);
}

/**
 * One registry behind three surfaces: the activity rail renders from it, the
 * chords are bound from it, and every entry is contributed to the Spotlight
 * palette the app already ships — which the board previously never touched, so
 * none of its thirteen dock actions were searchable.
 */
export function useBoardCommands(commands: IBoardCommand[]): void {
	const registerDynamicItems = useSpotlightStore(
		(state) => state.registerDynamicItems,
	);
	const unregisterDynamicItems = useSpotlightStore(
		(state) => state.unregisterDynamicItems,
	);
	const registerGroup = useSpotlightStore((state) => state.registerGroup);

	const commandsRef = useRef(commands);
	commandsRef.current = commands;

	const enabled = useMemo(
		() => commands.filter((command) => command.when !== false),
		[commands],
	);

	useEffect(() => {
		registerGroup({ id: SPOTLIGHT_GROUP, label: "Board", priority: 5 });
		registerDynamicItems(
			SPOTLIGHT_SOURCE,
			enabled.map((command) => ({
				id: `board:${command.id}`,
				type: "action" as const,
				label: command.title,
				description: command.description,
				icon: command.icon,
				keywords: command.keywords,
				shortcut: command.shortcut
					? formatShortcut(command.shortcut)
					: undefined,
				group: SPOTLIGHT_GROUP,
				action: () => command.run(),
			})),
		);
		return () => unregisterDynamicItems(SPOTLIGHT_SOURCE);
	}, [enabled, registerDynamicItems, unregisterDynamicItems, registerGroup]);

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (isEditableTarget(event.target)) return;
			for (const command of commandsRef.current) {
				if (!command.shortcut || command.when === false) continue;
				if (!matches(event, command.shortcut)) continue;
				event.preventDefault();
				event.stopPropagation();
				command.run();
				return;
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);
}
