"use client";

import { BotsView } from "@flow-like/flow-like-ui/components/bots-tab";

import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { useBackend } from "@flow-like/flow-like-ui/state/backend-state";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { useAuth } from "react-oidc-context";
import { NativeVoiceInput } from "../../components/native-voice-input";

export default function BotsPage() {
	return (
		<Suspense fallback={null}>
			<BotsPageContent />
		</Suspense>
	);
}

/**
 * Drops only `?voice=`, so the dock keeps its bot or conversation. Native history applies at once,
 * which lets a reveal queued by Send in the same click build its URL without the parameter.
 */
function closeVoiceInput() {
	const params = new URLSearchParams(window.location.search);
	params.delete("voice");
	const query = params.toString();
	const { pathname } = window.location;
	window.history.replaceState(
		null,
		"",
		query ? `${pathname}?${query}` : pathname,
	);
}

function BotsPageContent() {
	const params = useSearchParams();
	const backend = useBackend();
	const auth = useAuth();
	const scope = JSON.stringify([
		getApiOrigin(backend.profile),
		backend.profile?.id ?? "",
		(auth.isAuthenticated ? auth.user?.profile.sub : undefined) ?? "local",
	]);
	return (
		<main
			className="flex flex-col flex-1 w-full min-h-0 overflow-hidden bg-background"
			data-fl-chat-bottom-nav
		>
			{params.get("voice") === "1" && (
				<NativeVoiceInput key={scope} scope={scope} onClose={closeVoiceInput} />
			)}
			<BotsView />
		</main>
	);
}
