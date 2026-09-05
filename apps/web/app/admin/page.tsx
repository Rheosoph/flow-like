"use client";

import { AdminDashboardPage } from "@flow-like/flow-like-ui";
import { useAuth } from "react-oidc-context";

export default function AdminPage() {
	const auth = useAuth();
	return (
		<AdminDashboardPage
			infoEnabled={Boolean(auth?.isAuthenticated)}
			infoDependencyKey={[auth?.user?.profile?.sub, auth?.isAuthenticated]}
		/>
	);
}
