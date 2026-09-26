"use client";

import {
	PackageWorkspace,
	PackageWorkspaceSkeleton,
} from "@flow-like/flow-like-ui";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { useAuth } from "react-oidc-context";
import { fetcher } from "../../../lib/api";

function WorkspaceRoute() {
	const auth = useAuth();
	const packageId = useSearchParams().get("id") ?? undefined;
	return (
		<PackageWorkspace packageId={packageId} fetcher={fetcher} auth={auth} />
	);
}

export default function Page() {
	return (
		<Suspense fallback={<PackageWorkspaceSkeleton />}>
			<WorkspaceRoute />
		</Suspense>
	);
}
