"use client";

import { PackageWorkspaceSkeleton } from "@flow-like/flow-like-ui/components/store/package-workspace/package-workspace";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { DesktopPackageWorkspace } from "../../../components/packages/desktop-package-workspace";

function WorkspaceRoute() {
	const searchParams = useSearchParams();
	return (
		<DesktopPackageWorkspace
			id={searchParams.get("id") ?? undefined}
			project={searchParams.get("project") ?? undefined}
		/>
	);
}

export default function Page() {
	return (
		<Suspense fallback={<PackageWorkspaceSkeleton />}>
			<WorkspaceRoute />
		</Suspense>
	);
}
