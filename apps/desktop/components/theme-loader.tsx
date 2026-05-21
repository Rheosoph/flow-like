"use client";

import { loadTheme, useBackend, useInvoke } from "@flow-like/flow-like-ui";
import { useEffect } from "react";

export function ThemeLoader() {
	const backend = useBackend();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	useEffect(() => {
		if (!profile.data?.theme) {
			loadTheme({
				light: {},
				dark: {},
			});
			return;
		}

		loadTheme(profile.data.theme);
	}, [profile.data]);

	return null;
}
