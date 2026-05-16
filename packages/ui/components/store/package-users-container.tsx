"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import type {
	InviteUserRequest,
	PackageUser,
	UpdateUserPermissionRequest,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import { PackageUsersTab } from "./package-users-tab";

export interface PackageUsersContainerProps {
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
	currentUserPermission: number;
}

export function PackageUsersContainer({
	packageId,
	fetcher,
	auth,
	currentUserPermission,
}: PackageUsersContainerProps) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const queryClient = useQueryClient();
	const queryKey = ["package-users", packageId];

	const { data: users = [], isLoading } = useQuery<PackageUser[]>({
		queryKey,
		queryFn: () =>
			fetcher<PackageUser[]>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/users`,
				{ method: "GET" },
				auth,
			),
		enabled: !!profile.data,
	});

	const invite = useMutation({
		mutationFn: (request: InviteUserRequest) =>
			fetcher<void>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/users/invite`,
				{
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(request),
				},
				auth,
			),
		onSuccess: () => {
			toast.success("Invitation sent");
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () => toast.error("Failed to send invitation"),
	});

	const updatePermission = useMutation({
		mutationFn: ({
			userId,
			request,
		}: { userId: string; request: UpdateUserPermissionRequest }) =>
			fetcher<void>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/users/${userId}`,
				{
					method: "PUT",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(request),
				},
				auth,
			),
		onSuccess: () => {
			toast.success("Permission updated");
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () => toast.error("Failed to update permission"),
	});

	const removeUser = useMutation({
		mutationFn: (userId: string) =>
			fetcher<void>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/users/${userId}`,
				{ method: "DELETE" },
				auth,
			),
		onSuccess: () => {
			toast.success("User removed");
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () => toast.error("Failed to remove user"),
	});

	const isMutating =
		invite.isPending || updatePermission.isPending || removeUser.isPending;

	return (
		<PackageUsersTab
			packageId={packageId}
			users={users}
			currentUserPermission={currentUserPermission}
			isLoading={isLoading}
			onInvite={(req) => invite.mutate(req)}
			onUpdatePermission={(userId, request) =>
				updatePermission.mutate({ userId, request })
			}
			onRemoveUser={(userId) => removeUser.mutate(userId)}
			isMutating={isMutating}
		/>
	);
}
