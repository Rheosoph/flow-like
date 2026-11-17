import { invoke } from "@tauri-apps/api/core";
import type {
	HostTurn,
	NotchLayout,
	NotchRect,
	NotchSettings,
	NotchTab,
	NotchWindow,
	PerchSpec,
} from "./protocol";

export const notchCommands = {
	layout: () => invoke<NotchLayout>("bot_notch_layout"),
	settings: () => invoke<NotchSettings>("bot_notch_settings"),
	setEnabled: (enabled: boolean) =>
		invoke<void>("bot_notch_set_enabled", { enabled }),
	setHiddenBots: (keys: readonly string[]) =>
		invoke<void>("bot_notch_set_hidden_bots", { keys }),
	setVisible: (visible: boolean) =>
		invoke<void>("bot_notch_set_visible", { visible }),
	setHitboxes: (
		rects: readonly NotchRect[],
		capture: boolean,
		proximity: number,
	) => invoke<void>("bot_notch_set_hitboxes", { rects, capture, proximity }),
	windowAt: (x: number, y: number) =>
		invoke<NotchWindow | null>("bot_notch_window_at", { x, y }),
	activeTab: (windowId: string) =>
		invoke<NotchTab | null>("bot_notch_active_tab", { windowId }),
	setPerches: (perches: readonly PerchSpec[]) =>
		invoke<void>("bot_notch_set_perches", { perches }),
	focusWindow: (windowId: string) =>
		invoke<void>("bot_notch_focus_window", { windowId }),
	hostId: () => invoke<string>("bot_notch_host_id"),
	hostTurns: () => invoke<HostTurn[]>("bot_host_turns"),
};
