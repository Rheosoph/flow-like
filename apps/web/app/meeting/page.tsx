import { ConnectedMeetingPage } from "@flow-like/flow-like-ui/components/meetings/connected-meeting-page";
import type { Metadata } from "next";
import { Suspense } from "react";

export const metadata: Metadata = {
	title: "Meeting",
	robots: { index: false, follow: false },
	referrer: "no-referrer",
};

export default function Page() {
	return (
		<Suspense fallback={null}>
			<ConnectedMeetingPage />
		</Suspense>
	);
}
