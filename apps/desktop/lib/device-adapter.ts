import {
	DeviceError,
	type DeviceReply,
	registerDeviceAdapter,
} from "@flow-like/flow-like-ui/lib/device-bridge";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
	browserLocationAvailable,
	normalizeLocationOptions,
	readBrowserLocation,
	UNSUPPORTED_GEOFENCE_STATUS,
} from "@flow-like/flow-like-ui/lib/location";

function locationError(error: unknown): DeviceError {
	if (error instanceof DeviceError) return error;
	const message =
		typeof error === "string"
			? error
			: error instanceof Error
				? error.message
				: "Location could not be determined";
	const match = message.match(/^\[?([a-z_]+)\]?:\s*(.*)$/);
	return new DeviceError(
		match?.[1] || "position_unavailable",
		match?.[2] || message,
	);
}

export function installNativeDeviceAdapter(): () => void {
	const apple = /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);
	const unregister = registerDeviceAdapter(
		async (command, args, context) => {
			if (context.signal?.aborted)
				throw new DeviceError("cancelled", "The device request ended");
			if (command === "location.geofencePermission") {
				if (!apple) return UNSUPPORTED_GEOFENCE_STATUS;
				try {
					return await invoke("native_geofence_permission", {
						mode: args.mode,
					});
				} catch (error) {
					throw locationError(error);
				}
			}
			if (command === "location.current") {
				const options = normalizeLocationOptions({
					...args,
					requestDeadline: context.deadline,
				});
				if (!apple) return readBrowserLocation(options, context);
				const requestId = crypto.randomUUID();
				const cancel = () => {
					void invoke("native_cancel_location", { requestId }).catch(() => {});
				};
				context.signal?.addEventListener("abort", cancel, { once: true });
				try {
					if (context.signal?.aborted)
						throw new DeviceError("cancelled", "The location request ended");
					return await invoke("native_get_location", { requestId, options });
				} catch (error) {
					throw locationError(error);
				} finally {
					context.signal?.removeEventListener("abort", cancel);
				}
			}
			if (command === "clipboard.write") {
				const result = await invoke<DeviceReply>("native_write_clipboard", {
					payload: { ...args, requestDeadline: context.deadline },
				});
				if (!result.ok)
					throw new DeviceError(result.error.code, result.error.message);
				return result.value;
			}
			throw new DeviceError(
				"unsupported",
				`Device operation ${command} is unavailable`,
			);
		},
		{ location: apple ? "native" : browserLocationAvailable() },
	);
	let disposed = false;
	const unlisteners: Array<() => void> = [];
	const inactive = () =>
		window.dispatchEvent(new CustomEvent("flow-like:device-inactive"));
	const locationBackground = () =>
		window.dispatchEvent(new CustomEvent("flow-like:location-background"));
	void listen("native-location-background", locationBackground).then(
		(unlisten) => {
			if (disposed) unlisten();
			else unlisteners.push(unlisten);
		},
	);
	for (const name of ["native-inactive", "tauri://blur"]) {
		void listen(name, inactive).then((unlisten) => {
			if (disposed) unlisten();
			else unlisteners.push(unlisten);
		});
	}
	return () => {
		disposed = true;
		unregister();
		for (const unlisten of unlisteners) unlisten();
		inactive();
		locationBackground();
	};
}
