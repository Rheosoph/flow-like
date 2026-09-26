"use client";

import {
	LibraryPackages,
	PackagesHubPage,
	RegistryMinePackages,
} from "@flow-like/flow-like-ui";
import { type ReactNode, useCallback } from "react";
import { useAuth } from "react-oidc-context";
import { fetcher } from "../../../lib/api";

export default function Page() {
	const auth = useAuth();
	const mine = useCallback(
		(navigation: ReactNode) => (
			<RegistryMinePackages auth={auth} navigation={navigation} />
		),
		[auth],
	);
	const library = useCallback(
		(navigation: ReactNode) => (
			<LibraryPackages auth={auth} navigation={navigation} />
		),
		[auth],
	);

	return (
		<PackagesHubPage
			fetcher={fetcher}
			auth={auth}
			mine={mine}
			library={library}
		/>
	);
}
