"use client";

import { GlobalChatView } from "@flow-like/flow-like-ui/components/global-chat/global-chat-view";

import { useRouter, useSearchParams } from "next/navigation";
import { NativeVoiceInput } from "../../components/native-voice-input";
import { useBackend } from "@flow-like/flow-like-ui/state/backend-state";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { useAuth } from "react-oidc-context";
import { Suspense } from "react";

export default function ChatPage() {
	return (
		<Suspense fallback={null}>
			<ChatPageContent />
		</Suspense>
	);
}

function ChatPageContent() {
	const router = useRouter();
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
				<NativeVoiceInput
					key={scope}
					scope={scope}
					onClose={() => router.replace("/chat")}
				/>
			)}
			<GlobalChatView />
		</main>
	);
}
