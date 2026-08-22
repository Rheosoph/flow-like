export function isTauriRuntime(): boolean {
	if (typeof window === "undefined") return false;
	const tauriWindow = window as Window & {
		__TAURI__?: unknown;
		__TAURI_IPC__?: unknown;
		__TAURI_INTERNALS__?: unknown;
	};
	return Boolean(
		tauriWindow.__TAURI__ ||
			tauriWindow.__TAURI_IPC__ ||
			tauriWindow.__TAURI_INTERNALS__,
	);
}

/** Coarse platform label for anything the hub groups by OS (telemetry, captured failures). */
export function desktopPlatform(): string {
	if (typeof navigator === "undefined") return "desktop";
	const ua = navigator.userAgent.toLowerCase();
	if (ua.includes("android")) return "android";
	if (/ipad|iphone|ipod/.test(ua)) return "ios";
	if (ua.includes("mac")) return "macos";
	if (ua.includes("win")) return "windows";
	if (ua.includes("linux")) return "linux";
	return "desktop";
}

export function isIOSDevice(): boolean {
	if (typeof navigator === "undefined") return false;
	return (
		/iPad|iPhone|iPod/.test(navigator.userAgent) ||
		(navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1)
	);
}

export function isAndroidDevice(): boolean {
	if (typeof navigator === "undefined") return false;
	return /Android/i.test(navigator.userAgent);
}

export function isMobileDevice(): boolean {
	return isIOSDevice() || isAndroidDevice();
}

export function isIosTauriRuntime(): boolean {
	return isTauriRuntime() && isIOSDevice();
}
