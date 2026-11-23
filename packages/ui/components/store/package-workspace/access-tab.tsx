"use client";

import { useRouter } from "next/navigation";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import { PackageAccessTab } from "../package-access-tab";
import { PackageUsersTab } from "../package-users-tab";
import { usePackageAccess } from "./use-package-access";

const MINE_HREF = "/store/packages?tab=mine";

export function AccessTab({
	packageId,
	permission,
	fetcher,
	auth,
}: Readonly<{
	packageId: string;
	permission: number;
	fetcher: GenericFetcher;
	auth?: unknown;
}>) {
	const backend = useBackend();
	const router = useRouter();
	const self = useInvoke(backend.userState.getInfo, backend.userState, []);
	const access = usePackageAccess(packageId, fetcher, auth);

	return (
		<div className="flex flex-col gap-4">
			<PackageUsersTab
				access={access}
				callerPermission={permission}
				selfId={self.data?.id}
				onLeft={() => router.push(MINE_HREF)}
			/>
			<PackageAccessTab access={access} />
		</div>
	);
}
