"use client";
import { FlowWrapper } from "@tm9657/flow-like-ui/components/flow/flow-wrapper";
import "@xyflow/react/dist/style.css";
import { Video } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { RecordingDock } from "../../components/rpa";

export default function FlowEditPage() {
	const searchParams = useSearchParams();
	const [showRecording, setShowRecording] = useState(false);
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
			extraDockItems={[
				{
					icon: <Video className={showRecording ? "text-red-500" : ""} />,
					title: "Record Actions",
					separator: "left",
					onClick: () => setShowRecording((prev) => !prev),
				},
			]}
			renderOverlay={() =>
				showRecording ? (
					<RecordingDock
						boardId={boardId}
						appId={appId || undefined}
						token={auth.user?.access_token}
						version={version}
						onClose={() => setShowRecording(false)}
					/>
				) : null
			}
		/>
	);
}
