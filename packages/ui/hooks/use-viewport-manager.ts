import { useReactFlow } from "@xyflow/react";
import { useCallback, useEffect, useRef } from "react";
import { viewportDb, viewportKey } from "../db/viewport-db";

interface UseViewportManagerProps {
	appId: string;
	boardId: string;
	layerPath: string | undefined;
	nodesLength: number;
}

/** Call to end a hold taken with `holdViewport`. Safe to call more than once. */
export type ViewportHold = () => void;

export function useViewportManager({
	appId,
	boardId,
	layerPath,
	nodesLength,
}: UseViewportManagerProps) {
	const { getViewport, setViewport, fitView } = useReactFlow();
	const holds = useRef(0);

	const saveViewport = useCallback(async () => {
		try {
			const vp = getViewport();
			await viewportDb.viewports.put({
				id: viewportKey(appId, boardId, layerPath),
				appId,
				boardId,
				layerPath: layerPath ?? "root",
				x: vp.x,
				y: vp.y,
				zoom: vp.zoom,
				updatedAt: Date.now(),
			});
		} catch {
			// no-op
		}
	}, [appId, boardId, layerPath, getViewport]);

	/**
	 * Taken by navigation that positions the viewport itself (go-to-node, jump-to-peer).
	 * Entering a layer changes both `layerPath` and `nodesLength`, so the restore below
	 * fires — asynchronously — and would otherwise land after the caller's `fitView` and
	 * drop the user at the layer's last-saved viewport instead of on the thing they
	 * navigated to.
	 */
	const holdViewport = useCallback((): ViewportHold => {
		holds.current += 1;
		let released = false;
		return () => {
			if (released) return;
			released = true;
			holds.current = Math.max(0, holds.current - 1);
		};
	}, []);

	useEffect(() => {
		let active = true;

		const restore = async () => {
			if (holds.current > 0) return;
			const rec = await viewportDb.viewports.get(
				viewportKey(appId, boardId, layerPath),
			);
			// Re-checked after the await: the hold is usually taken in the same tick that
			// changed `layerPath`, which is what scheduled this effect.
			if (!active || holds.current > 0) return;

			if (rec) {
				setViewport({ x: rec.x, y: rec.y, zoom: rec.zoom });
			} else {
				fitView({ duration: 300 });
			}
		};

		restore();

		return () => {
			active = false;
		};
	}, [appId, boardId, layerPath, setViewport, fitView, nodesLength]);

	return { saveViewport, holdViewport };
}
