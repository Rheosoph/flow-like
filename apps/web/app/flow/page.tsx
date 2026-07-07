"use client";
import { FlowWrapper } from "@flow-like/flow-like-ui/components/flow/flow-wrapper";
import "@xyflow/react/dist/style.css";
import { useSearchParams } from "next/navigation";
import { useMemo } from "react";
import { useAuth } from "react-oidc-context";

export default function FlowEditPage() {
	const searchParams = useSearchParams();
	const auth = useAuth();
	const { boardId, appId, nodeId, version } = useMemo(() => {
		const boardId = searchParams.get("id") ?? "";
		const appId = searchParams.get("app") ?? "";
		const nodeId = searchParams.get("node") ?? undefined;
		let version: any = searchParams.get("version") ?? undefined;
		if (version)
			version = version.split("_").map(Number) as [number, number, number];
		return { boardId, appId, nodeId, version };
	}, [searchParams]);

	if (boardId === "") return <p>Board not found...</p>;
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
