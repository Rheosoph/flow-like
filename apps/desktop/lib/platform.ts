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
