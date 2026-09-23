import { Suspense } from "react";
import { HostedFrontend } from "../../components/hosted-frontend";

export default function HostedPage() {
	return (
		<Suspense fallback={<output>Loading interface…</output>}>
			<HostedFrontend />
		</Suspense>
	);
}
