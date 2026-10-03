"use client";

import { useCallback, useEffect, useRef, useState } from "react";

function copyWithTextarea(text: string): boolean {
	if (typeof document === "undefined") return false;
	const area = document.createElement("textarea");
	area.value = text;
	area.setAttribute("readonly", "");
	area.style.position = "absolute";
	area.style.left = "-9999px";
	document.body.appendChild(area);
	area.select();
	try {
		return document.execCommand?.("copy") ?? false;
	} catch {
		return false;
	} finally {
		area.remove();
	}
}

/** Clipboard write inside the click handler, falling back to a hidden textarea (SPEC §4.19). */
export async function copyText(text: string): Promise<boolean> {
	try {
		if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
			await navigator.clipboard.writeText(text);
			return true;
		}
	} catch {
		/* fall through to the textarea path */
	}
	return copyWithTextarea(text);
}

/** `copied` stays true for `resetMs` after a successful copy, for the "Copied" icon swap. */
export function useCopy(resetMs = 1500) {
	const [copied, setCopied] = useState(false);
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(
		() => () => {
			if (timer.current) clearTimeout(timer.current);
		},
		[],
	);

	const copy = useCallback(
		async (text: string) => {
			const ok = await copyText(text);
			if (!ok) return false;
			setCopied(true);
			if (timer.current) clearTimeout(timer.current);
			timer.current = setTimeout(() => setCopied(false), resetMs);
			return true;
		},
		[resetMs],
	);

	return { copied, copy };
}
