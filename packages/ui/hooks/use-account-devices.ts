"use client";

import { useQuery } from "@tanstack/react-query";
import { listDevices } from "../lib/devices";
import { useBackend } from "../state/backend-state";
import { useInvoke } from "./use-invoke";

/** The signed-in account's devices as the hub lists them; shared by everything in the bots UI that names a device. */
export function useAccountDevices(enabled = true, refresh = false) {
	const backend = useBackend();
	const settings = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = settings.data?.hub_profile;
	const devices = useQuery({
		queryKey: ["bots", "editor", "devices", profile?.id ?? ""],
		queryFn: () => {
			if (!profile) throw new Error("No profile to list devices for");
			return listDevices(backend.apiState, profile);
		},
		enabled: enabled && !!profile,
		staleTime: 30_000,
		refetchInterval: refresh ? 30_000 : false,
	});
	return {
		devices: devices.data,
		isLoading: settings.isLoading || devices.isLoading,
		isError: devices.isError,
		refetch: devices.refetch,
		isFetching: devices.isFetching,
	};
}
