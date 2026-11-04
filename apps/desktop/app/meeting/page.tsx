import { ConnectedMeetingPage } from "@flow-like/flow-like-ui/components/meetings/connected-meeting-page";
import { Suspense } from "react";

export default function Page() {
	return (
		<Suspense fallback={null}>
			<ConnectedMeetingPage />
		</Suspense>
	);
}
