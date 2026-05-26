"use client";

import { PackagesStorePage } from "@flow-like/flow-like-ui";
import { useAuth } from "react-oidc-context";
import { fetcher } from "../../../lib/api";

export default function Page() {
	const auth = useAuth();
	return <PackagesStorePage fetcher={fetcher} auth={auth} />;
}
