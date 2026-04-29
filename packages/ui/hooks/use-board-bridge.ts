"use client";
import { type RefObject, useCallback, useEffect, useRef } from "react";
import {
	BOARD_BRIDGE_PROTOCOL,
	type BoardSnapshot,
	isBoardBridgeMessage,
} from "../lib/learn/board-bridge";

/**
 * Lesson-side bridge: requests a board snapshot from a child iframe.
 * Returns a stable function `requestBoardState()` that resolves with the
 * snapshot or rejects on timeout. Tracks pending requests by id so callers
 * may invoke it concurrently.
 */
export function useBoardBridge(
	iframeRef: RefObject<HTMLIFrameElement | null>,
	options: { readonly timeoutMs?: number } = {},
): { readonly requestBoardState: () => Promise<BoardSnapshot> } {
	const timeoutMs = options.timeoutMs ?? 4000;
	const pending = useRef(
		new Map<
			string,
			{
				readonly resolve: (s: BoardSnapshot) => void;
				readonly reject: (e: Error) => void;
			}
		>(),
	);

	useEffect(() => {
		const handle = (event: MessageEvent) => {
			if (!isBoardBridgeMessage(event.data)) return;
			if (event.data.type !== "FL_BOARD_STATE") return;
			const entry = pending.current.get(event.data.requestId);
			if (!entry) return;
			pending.current.delete(event.data.requestId);
			entry.resolve(event.data.snapshot);
		};
		window.addEventListener("message", handle);
		return () => window.removeEventListener("message", handle);
	}, []);

	const requestBoardState = useCallback((): Promise<BoardSnapshot> => {
		const iframe = iframeRef.current;
		if (!iframe?.contentWindow) {
			return Promise.reject(new Error("iframe is not attached"));
		}
		const requestId =
			typeof crypto !== "undefined" && "randomUUID" in crypto
				? crypto.randomUUID()
				: `${Date.now()}-${Math.random().toString(36).slice(2)}`;
		return new Promise<BoardSnapshot>((resolve, reject) => {
			const timer = window.setTimeout(() => {
				pending.current.delete(requestId);
				reject(
					new Error(
						"Timed out waiting for board state. Is the embedded page mounted with <BoardBridgeResponder/>?",
					),
				);
			}, timeoutMs);
			pending.current.set(requestId, {
				resolve: (snapshot) => {
					window.clearTimeout(timer);
					resolve(snapshot);
				},
				reject: (err) => {
					window.clearTimeout(timer);
					reject(err);
				},
			});
			iframe.contentWindow?.postMessage(
				{
					protocol: BOARD_BRIDGE_PROTOCOL,
					type: "FL_REQUEST_BOARD_STATE",
					requestId,
				},
				"*",
			);
		});
	}, [iframeRef, timeoutMs]);

	return { requestBoardState };
}
