"use client";
import { useEffect } from "react";
import {
	BOARD_BRIDGE_NATIVE_EVENT,
	BOARD_BRIDGE_PROTOCOL,
	type BoardBridgeMessage,
	type BoardBridgeNativeRequestDetail,
	type BoardSnapshot,
	isBoardBridgeMessage,
} from "../../lib/learn/board-bridge";

interface BoardBridgeResponderProps {
	readonly snapshot: () => BoardSnapshot | null;
	/** Optional ready announcement details; helps lesson runtime know we're alive. */
	readonly announce?: { readonly appId?: string; readonly boardId?: string };
}

/**
 * Mount inside an embedded board page (e.g. /use, /flow) to expose the board
 * state to a parent lesson runtime via window.postMessage. No-op when not
 * embedded in an iframe.
 */
export function BoardBridgeResponder({
	snapshot,
	announce,
}: BoardBridgeResponderProps) {
	useEffect(() => {
		if (typeof window === "undefined") return;

		function send(msg: BoardBridgeMessage) {
			window.parent.postMessage(msg, "*");
		}

		if (window.parent !== window) {
			send({
				protocol: BOARD_BRIDGE_PROTOCOL,
				type: "FL_BOARD_BRIDGE_READY",
				appId: announce?.appId,
				boardId: announce?.boardId,
			});
		}

		const handle = (event: MessageEvent) => {
			if (!isBoardBridgeMessage(event.data)) return;
			if (event.data.type !== "FL_REQUEST_BOARD_STATE") return;
			const snap = snapshot();
			if (!snap) return;
			send({
				protocol: BOARD_BRIDGE_PROTOCOL,
				type: "FL_BOARD_STATE",
				requestId: event.data.requestId,
				snapshot: snap,
			});
		};

		const handleNative = (event: Event) => {
			const detail = (event as CustomEvent<BoardBridgeNativeRequestDetail>)
				.detail;
			const snap = snapshot();
			if (!snap) {
				detail?.reject?.(new Error("Board state is not ready."));
				return;
			}
			detail?.resolve?.(snap);
		};

		window.addEventListener("message", handle);
		window.addEventListener(BOARD_BRIDGE_NATIVE_EVENT, handleNative);
		return () => {
			window.removeEventListener("message", handle);
			window.removeEventListener(BOARD_BRIDGE_NATIVE_EVENT, handleNative);
		};
	}, [snapshot, announce?.appId, announce?.boardId]);

	return null;
}
