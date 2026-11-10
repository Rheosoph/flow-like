"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { capturePageSnapshot } from "../../lib/page-snapshot";
import { useAssistantSurface } from "../../state/assistant-surface";
import type { ComposerAttachSource } from "../../state/composer-capture";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";

/**
 * "Screenshot" of the view behind the floating chat. Boards draw on a WebGL canvas that a DOM
 * capture renders blank, so the source stays hidden while one is open.
 */
export function usePageSnapshotSource(): ComposerAttachSource | undefined {
	const { t } = useTranslation();
	const floating = useGlobalChatStore((state) => state.mode === "overlay");
	const onBoard = useAssistantSurface((state) => Boolean(state.boardSurface));
	return useMemo(
		() =>
			floating && !onBoard
				? {
						kind: "page_snapshot",
						attach: async () => {
							try {
								return { files: [await capturePageSnapshot()] };
							} catch (error) {
								console.warn("[PageSnapshot] capture failed:", error);
								throw new Error(
									t("pageSnapshotFailed", "Could not capture this page."),
								);
							}
						},
					}
				: undefined,
		[floating, onBoard, t],
	);
}
