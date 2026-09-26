"use client";

import type { GenericFetcher } from "../../pages/store/store-package-detail";
import { PackageAccessTab } from "../package-access-tab";
import { PackageUsersContainer } from "../package-users-container";

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
	return (
		<div className="flex flex-col gap-4">
			<PackageUsersContainer
				packageId={packageId}
				fetcher={fetcher}
				auth={auth}
				currentUserPermission={permission}
			/>
			<PackageAccessTab packageId={packageId} fetcher={fetcher} auth={auth} />
		</div>
	);
}
