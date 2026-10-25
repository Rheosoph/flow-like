"use client";
import { FlowWrapper } from "@flow-like/flow-like-ui/components/flow/flow-wrapper";
import { disposeNodeSuggestionEngine } from "@flow-like/flow-like-ui/lib/node-suggestions/engine";
import { useTabDormancy } from "@flow-like/flow-like-ui/lib/tab-dormancy";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import { useRunExecutionStore } from "@flow-like/flow-like-ui/state/run-execution-state";
import { useTranslation } from "@flow-like/locales";
import "@xyflow/react/dist/style.css";
import { useSearchParams } from "next/navigation";
import { useEffect, useMemo } from "react";
import { useAuth } from "react-oidc-context";

const BOARD_DORMANCY_AFTER_MS = 3 * 60_000;
const MODAL_CONTENT =
	'[data-slot="dialog-content"],[data-slot="alert-dialog-content"],[data-slot="sheet-content"]';

/** Unmounting would cut off a run or a FlowPilot edit, or drop what an open dialog holds. */
function boardTabBusy() {
	return (
		useRunExecutionStore.getState().runs.size > 0 ||
		useGlobalChatStore.getState().isStreaming ||
		document.querySelector(MODAL_CONTENT) !== null
	);
}

export default function FlowEditPage() {
	const { t } = useTranslation("common");
	const searchParams = useSearchParams();
	const auth = useAuth();
	const dormant = useTabDormancy(BOARD_DORMANCY_AFTER_MS, boardTabBusy);
	useEffect(() => {
		if (dormant) disposeNodeSuggestionEngine();
	}, [dormant]);
	const { boardId, appId, nodeId, version } = useMemo(() => {
		const boardId = searchParams.get("id") ?? "";
		const appId = searchParams.get("app") ?? "";
		const nodeId = searchParams.get("node") ?? undefined;
		let version: any = searchParams.get("version") ?? undefined;
		if (version)
			version = version.split("_").map(Number) as [number, number, number];
		return { boardId, appId, nodeId, version };
	}, [searchParams]);

	if (boardId === "") return <p>{t("boardNotFound", "Board not found...")}</p>;
	// The tab is hidden, so nothing shows; when it is shown the board remounts from the query cache.
	if (dormant) return null;
	return (
		<FlowWrapper
			boardId={boardId}
			appId={appId}
			nodeId={nodeId}
			version={version}
			sub={auth.user?.profile?.sub}
			externalAssistant
		/>
	);
}
