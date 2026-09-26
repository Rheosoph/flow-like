"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { apiErrorMessage } from "../../lib/api-error";
import { asArray } from "../../lib/response-shape";
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
	const { t } = useTranslation("store");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const queryClient = useQueryClient();
	const queryKey = ["package-users", packageId];
	const hubProfile = profile.data?.hub_profile;

	function usersRequest<T>(path: string, init: RequestInit): Promise<T> {
		const url = `registry/package/${packageId}/users${path}`;
		if (!hubProfile) {
			return Promise.reject(
				new Error(`Cannot call ${url}: the hub profile is not loaded`),
			);
		}
		return fetcher<T>(hubProfile, url, init, auth);
	}

	const { data: userData, isLoading } = useQuery<PackageUser[]>({
		queryKey,
		queryFn: () => usersRequest<PackageUser[]>("", { method: "GET" }),
		enabled: !!hubProfile,
	});
	const users = asArray(userData);

	const invite = useMutation({
		mutationFn: (request: InviteUserRequest) =>
			usersRequest<void>("/invite", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify(request),
			}),
		onSuccess: () => {
			toast.success(t("invitationSent", "Invitation sent"));
			queryClient.invalidateQueries({ queryKey });
		},
		onError: (error) =>
			toast.error(
				apiErrorMessage(
					error,
					t("failedToSendInvitation", "Failed to send invitation"),
				),
			),
	});

	const updatePermission = useMutation({
		mutationFn: ({
			userId,
			request,
		}: { userId: string; request: UpdateUserPermissionRequest }) =>
			usersRequest<void>(`/${userId}`, {
				method: "PATCH",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify(request),
			}),
		onSuccess: () => {
			toast.success(t("permissionUpdated", "Permission updated"));
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () =>
			toast.error(t("failedToUpdatePermission", "Failed to update permission")),
	});

	const removeUser = useMutation({
		mutationFn: (userId: string) =>
			usersRequest<void>(`/${userId}`, { method: "DELETE" }),
		onSuccess: () => {
			toast.success(t("userRemoved", "User removed"));
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () =>
			toast.error(t("failedToRemoveUser", "Failed to remove user")),
	});

	const isMutating =
		invite.isPending || updatePermission.isPending || removeUser.isPending;

	return (
		<PackageUsersTab
			packageId={packageId}
			users={users}
			currentUserPermission={currentUserPermission}
			isLoading={isLoading}
			onInvite={(req) =>
				invite.mutateAsync(req).then(
					() => true,
					() => false,
				)
			}
			onUpdatePermission={(userId, request) =>
				updatePermission.mutate({ userId, request })
			}
			onRemoveUser={(userId) => removeUser.mutate(userId)}
			isMutating={isMutating}
		/>
	);
}
