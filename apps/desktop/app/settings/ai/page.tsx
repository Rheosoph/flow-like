"use client";

import { AIModelPage, useBackend, useInvoke } from "@flow-like/flow-like-ui";

export default function SettingsAiPage() {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	if (!profile.data) {
		return null;
	}

	return <AIModelPage />;
}
