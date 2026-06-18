"use client";

import { LibraryPage } from "@flow-like/flow-like-ui";
import { useAuth } from "react-oidc-context";

export default function Page() {
	const auth = useAuth();

	return <LibraryPage isAuthenticated={auth.isAuthenticated} />;
}
