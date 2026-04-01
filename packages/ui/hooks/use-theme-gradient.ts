import { useSyncExternalStore } from "react";

const HUE_OFFSETS = [0, 30, 60, 90, 135, 180, 210, 250, 290, 330];
const FALLBACK_HUE = 26;

interface ThemeSnapshot {
	primaryHue: number;
	isDark: boolean;
}

export interface ThemeGradient {
	from: string;
	to: string;
	angle: number;
	opacity: number;
}

const SERVER_SNAPSHOT: ThemeSnapshot = {
	primaryHue: FALLBACK_HUE,
	isDark: true,
};
let _snapshot: ThemeSnapshot = { ...SERVER_SNAPSHOT };
const _listeners = new Set<() => void>();
let _observersReady = false;

function readTheme(): ThemeSnapshot {
	const isDark = document.documentElement.classList.contains("dark");
	let hue = FALLBACK_HUE;

	const dynamicStyle = document.getElementById("dynamic-theme");
	if (dynamicStyle?.textContent) {
		const section = isDark
			? (dynamicStyle.textContent.split(/\.dark\b/)[1] ?? "")
			: (dynamicStyle.textContent.split(":root")[1]?.split("}")[0] ?? "");
		const varMatch = section.match(
			/--primary\s*:\s*oklch\(\s*[\d.%]+\s+[\d.]+\s+([\d.]+)\s*\)/,
		);
		if (varMatch) {
			hue = Number.parseFloat(varMatch[1]);
		}
	}

	if (hue === FALLBACK_HUE) {
		for (const sheet of Array.from(document.styleSheets)) {
			try {
				for (const rule of Array.from(sheet.cssRules)) {
					const text = rule.cssText;
					if (!text.includes("--primary")) continue;
					const isRootRule = text.startsWith(":root");
					const isDarkRule = text.includes(".dark");
					if (
						(isDark && isDarkRule) ||
						(!isDark && isRootRule && !isDarkRule)
					) {
						const m = text.match(
							/--primary\s*:\s*oklch\(\s*[\d.%]+\s+[\d.]+\s+([\d.]+)\s*\)/,
						);
						if (m) {
							hue = Number.parseFloat(m[1]);
							break;
						}
					}
				}
			} catch {
				// Cross-origin stylesheet, skip
			}
			if (hue !== FALLBACK_HUE) break;
		}
	}

	return { primaryHue: hue, isDark };
}

function setupObservers() {
	if (_observersReady || typeof document === "undefined") return;
	_observersReady = true;
	_snapshot = readTheme();

	const refresh = () => {
		const next = readTheme();
		if (
			next.primaryHue !== _snapshot.primaryHue ||
			next.isDark !== _snapshot.isDark
		) {
			_snapshot = next;
			for (const l of _listeners) l();
		}
	};

	new MutationObserver(refresh).observe(document.documentElement, {
		attributes: true,
		attributeFilter: ["class"],
	});
	new MutationObserver(refresh).observe(document.head, {
		childList: true,
		subtree: true,
	});
}

function subscribeTheme(cb: () => void) {
	setupObservers();
	_listeners.add(cb);
	return () => {
		_listeners.delete(cb);
	};
}

function getSnapshot(): ThemeSnapshot {
	setupObservers();
	return _snapshot;
}

export function useThemeInfo(): ThemeSnapshot {
	return useSyncExternalStore(
		subscribeTheme,
		getSnapshot,
		() => SERVER_SNAPSHOT,
	);
}

export function hashToGradient(
	id: string,
	primaryHue: number,
	isDark: boolean,
): ThemeGradient {
	let hash = 0;
	for (let i = 0; i < id.length; i++) {
		hash = ((hash << 5) - hash + id.charCodeAt(i)) | 0;
	}
	const idx = Math.abs(hash) % HUE_OFFSETS.length;
	const hue1 = (primaryHue + HUE_OFFSETS[idx]) % 360;
	const hue2 = (hue1 + 30) % 360;
	const angle = (Math.abs(hash >> 8) % 4) * 45 + 120;

	const l1 = isDark ? 0.55 : 0.78;
	const l2 = isDark ? 0.65 : 0.85;
	const c1 = isDark ? 0.12 : 0.08;
	const c2 = isDark ? 0.1 : 0.06;

	return {
		from: `oklch(${l1} ${c1} ${hue1})`,
		to: `oklch(${l2} ${c2} ${hue2})`,
		angle,
		opacity: isDark ? 0.5 : 0.6,
	};
}
