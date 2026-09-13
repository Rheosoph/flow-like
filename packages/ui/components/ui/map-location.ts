"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { LocationFix } from "../../lib/location";

export interface MapLocationError {
	code: string;
	message: string;
}

export async function readMapLocation(
	appId: string | undefined,
	signal: AbortSignal,
): Promise<LocationFix> {
	const { executeDeviceCommand } = await import("../../lib/device-bridge");
	return (await executeDeviceCommand(
		"location.current",
		{},
		{
			appId: appId ?? "map",
			executionTarget: "local",
			userInitiated: true,
			signal,
		},
	)) as LocationFix;
}

/** One explicitly requested fix belongs to the currently visible map screen. */
export function useMapLocation({
	appId,
	scope,
	enabled,
	onLocation,
	onError,
	readLocation = readMapLocation,
}: {
	appId?: string;
	scope?: string;
	enabled: boolean;
	onLocation: (fix: LocationFix) => void | Promise<void>;
	onError?: (error: MapLocationError) => void;
	readLocation?: typeof readMapLocation;
}) {
	const elementRef = useRef<HTMLDivElement>(null);
	const request = useRef<AbortController | null>(null);
	const mounted = useRef(false);
	const latest = useRef({ enabled, onLocation, onError });
	latest.current = { enabled, onLocation, onError };
	const [waiting, setWaiting] = useState(false);
	const [error, setError] = useState<MapLocationError>();
	const visible = useCallback(() => {
		const element = elementRef.current;
		if (
			!mounted.current ||
			!latest.current.enabled ||
			!element?.isConnected ||
			document.visibilityState === "hidden" ||
			element.closest('[hidden], [aria-hidden="true"]')
		)
			return false;
		for (
			let node: HTMLElement | null = element;
			node;
			node = node.parentElement
		) {
			const style = getComputedStyle(node);
			if (style.display === "none" || style.visibility === "hidden")
				return false;
		}
		return true;
	}, []);

	useEffect(() => {
		mounted.current = true;
		setWaiting(false);
		setError(undefined);
		const cancel = () => {
			request.current?.abort();
			request.current = null;
			if (mounted.current) setWaiting(false);
		};
		const onVisibility = () => {
			if (!visible()) cancel();
		};
		document.addEventListener("visibilitychange", onVisibility);
		window.addEventListener("pagehide", cancel);
		window.addEventListener("flow-like:location-background", cancel);
		const observer = new MutationObserver(onVisibility);
		for (
			let node: HTMLElement | null = elementRef.current;
			node;
			node = node.parentElement
		) {
			observer.observe(node, {
				attributes: true,
				attributeFilter: ["hidden", "aria-hidden", "class", "style"],
			});
		}
		return () => {
			mounted.current = false;
			cancel();
			observer.disconnect();
			document.removeEventListener("visibilitychange", onVisibility);
			window.removeEventListener("pagehide", cancel);
			window.removeEventListener("flow-like:location-background", cancel);
		};
	}, [appId, scope, enabled, visible]);

	const locate = useCallback(async () => {
		if (request.current || !visible()) return;
		const controller = new AbortController();
		request.current = controller;
		setWaiting(true);
		setError(undefined);
		try {
			const fix = await readLocation(appId, controller.signal);
			if (controller.signal.aborted || !visible()) return;
			await latest.current.onLocation(fix);
		} catch (reason) {
			if (controller.signal.aborted || !visible()) return;
			const failure = {
				code:
					reason &&
					typeof reason === "object" &&
					"code" in reason &&
					typeof reason.code === "string"
						? reason.code
						: "location_failed",
				message:
					reason instanceof Error
						? reason.message
						: "The current location could not be read.",
			};
			setError(failure);
			latest.current.onError?.(failure);
		} finally {
			if (request.current === controller) {
				request.current = null;
				if (mounted.current) setWaiting(false);
			}
		}
	}, [appId, visible, readLocation]);
	return { elementRef, locate, waiting, error };
}
