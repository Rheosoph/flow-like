import type { DeviceCrypto } from "./types";

let loading: Promise<DeviceCrypto> | undefined;
export function loadDeviceCrypto(): Promise<DeviceCrypto> {
	if (typeof window === "undefined")
		return Promise.reject(
			new Error("Device management requires a browser session."),
		);
	if (!loading) {
		const modulePath = "/device-crypto/flow_like_device_crypto.js";
		loading = import(/* webpackIgnore: true */ /* @vite-ignore */ modulePath)
			.then(async (module: DeviceCrypto) => {
				await module.default();
				return module;
			})
			.catch(() => {
				loading = undefined;
				throw new Error(
					"Device cryptography could not be loaded. Refresh or check this app's installation.",
				);
			});
	}
	return loading;
}

export async function withPassword<T>(
	password: string,
	operation: (bytes: Uint8Array) => T | Promise<T>,
): Promise<T> {
	const bytes = new TextEncoder().encode(password);
	try {
		return await operation(bytes);
	} finally {
		bytes.fill(0);
	}
}

export function base64url(bytes: Uint8Array): string {
	let binary = "";
	for (let index = 0; index < bytes.length; index += 8192)
		binary += String.fromCharCode(...bytes.subarray(index, index + 8192));
	return btoa(binary)
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/u, "");
}

export function unbase64url(value: string, maximum = 32_768): Uint8Array {
	if (
		!/^[A-Za-z0-9_-]*$/u.test(value) ||
		value.length > Math.ceil((maximum * 4) / 3)
	)
		throw new Error("Invalid management encoding.");
	const bytes = Uint8Array.from(
		atob(value.replaceAll("-", "+").replaceAll("_", "/")),
		(c) => c.charCodeAt(0),
	);
	if (bytes.length > maximum || base64url(bytes) !== value)
		throw new Error("Invalid management encoding.");
	return bytes;
}
