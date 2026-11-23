"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryKey,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../lib/api-error";
import { asArray } from "../../../lib/response-shape";
import type {
	AccessRequest,
	PackagePendingInvitation,
	PackageUser,
} from "../../../lib/schema/wasm";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import {
	invalidatePackageLists,
	useHubProfile,
	usePackageUsers,
} from "./use-workspace-data";

export const packageInvitationsKey = (packageId: string): QueryKey => [
	"package-invitations",
	packageId,
];
export const packageAccessRequestsKey = (packageId: string): QueryKey => [
	"access-requests",
	packageId,
];

type Send = <T>(path: string, init?: RequestInit) => Promise<T>;

function useRegistrySend(
	packageId: string,
	fetcher: GenericFetcher,
	auth?: unknown,
) {
	const profile = useHubProfile();
	const hubProfile = profile.data?.hub_profile;
	const send: Send = (path, init = { method: "GET" }) => {
		const url = `registry/package/${packageId}${path}`;
		if (!hubProfile) {
			return Promise.reject(
				new Error(`Cannot call ${url}: the hub profile is not loaded`),
			);
		}
		return fetcher(hubProfile, url, init, auth);
	};
	return { send, ready: !!hubProfile };
}

const json = (method: string, body: unknown): RequestInit => ({
	method,
	headers: { "Content-Type": "application/json" },
	body: JSON.stringify(body),
});

export function usePackageAccess(
	packageId: string,
	fetcher: GenericFetcher,
	auth?: unknown,
) {
	const { t } = useTranslation("store");
	const queryClient = useQueryClient();
	const { send, ready } = useRegistrySend(packageId, fetcher, auth);

	const users = usePackageUsers(packageId, fetcher, auth);
	const invitations = useQuery<PackagePendingInvitation[]>({
		queryKey: packageInvitationsKey(packageId),
		queryFn: () => send<PackagePendingInvitation[]>("/invitations"),
		enabled: ready,
	});
	const requests = useQuery<AccessRequest[]>({
		queryKey: packageAccessRequestsKey(packageId),
		queryFn: () => send<AccessRequest[]>("/access"),
		enabled: ready,
	});

	const refresh = (...keys: QueryKey[]) => {
		for (const queryKey of keys)
			void queryClient.invalidateQueries({ queryKey });
	};
	const usersKey: QueryKey = ["package-users", packageId];
	const failed = (fallback: string) => (error: unknown) =>
		toast.error(apiErrorMessage(error, fallback));

	const invite = useMutation({
		mutationFn: (request: { inviteeId: string; permission: number }) =>
			send<void>("/users/invite", json("POST", request)),
		onSuccess: () => refresh(packageInvitationsKey(packageId)),
		onError: failed(t("failedToSendInvitation", "Failed to send invitation")),
	});

	const withdraw = useMutation({
		mutationFn: (invitationId: string) =>
			send<void>(`/invitations/${encodeURIComponent(invitationId)}`, {
				method: "DELETE",
			}),
		onSuccess: () => {
			toast.success(t("packageInvitationWithdrawn", "Invitation withdrawn"));
			refresh(packageInvitationsKey(packageId));
		},
		onError: (error) => {
			failed(
				t(
					"packageInvitationWithdrawFailed",
					"Couldn't withdraw the invitation",
				),
			)(error);
			refresh(packageInvitationsKey(packageId), usersKey);
		},
	});

	const setRole = useMutation({
		mutationFn: ({
			userId,
			permission,
		}: { userId: string; permission: number }) =>
			send<PackageUser>(
				`/users/${encodeURIComponent(userId)}`,
				json("PATCH", { permission }),
			),
		onSuccess: () => {
			toast.success(t("permissionUpdated", "Permission updated"));
			refresh(usersKey);
		},
		onError: (error) => {
			failed(t("failedToUpdatePermission", "Failed to update permission"))(
				error,
			);
			refresh(usersKey);
		},
	});

	const transfer = useMutation({
		mutationFn: ({
			userId,
			permission,
		}: { userId: string; permission: number }) =>
			send<PackageUser>(
				`/users/${encodeURIComponent(userId)}`,
				json("PATCH", { permission }),
			),
		onSuccess: () => {
			toast.success(t("packageOwnershipTransferred", "Ownership transferred"));
			refresh(usersKey);
			invalidatePackageLists(queryClient, packageId);
		},
		onError: failed(
			t("packageOwnershipTransferFailed", "Couldn't transfer ownership"),
		),
	});

	const remove = useMutation({
		mutationFn: (userId: string) =>
			send<void>(`/users/${encodeURIComponent(userId)}`, { method: "DELETE" }),
		onSuccess: () => {
			toast.success(t("userRemoved", "User removed"));
			refresh(usersKey);
		},
		onError: failed(t("failedToRemoveUser", "Failed to remove user")),
	});

	const leave = useMutation({
		mutationFn: (selfId: string) =>
			send<void>(`/users/${encodeURIComponent(selfId)}`, { method: "DELETE" }),
		onSuccess: () => {
			toast.success(t("packageLeft", "You left the package"));
			invalidatePackageLists(queryClient, packageId);
		},
		onError: failed(t("packageLeaveFailed", "Couldn't leave the package")),
	});

	const answerRequest = useMutation({
		mutationFn: ({ id, approve }: { id: string; approve: boolean }) =>
			send<void>(`/access/${encodeURIComponent(id)}`, {
				method: approve ? "POST" : "DELETE",
			}),
		onSuccess: (_, { approve }) => {
			toast.success(
				approve
					? t("accessRequestAccepted", "Access request accepted")
					: t("accessRequestRejected", "Access request rejected"),
			);
			refresh(packageAccessRequestsKey(packageId), usersKey);
		},
		onError: (error, { approve }) => {
			failed(
				approve
					? t("failedToAcceptRequest", "Failed to accept request")
					: t("failedToRejectRequest", "Failed to reject request"),
			)(error);
			refresh(packageAccessRequestsKey(packageId));
		},
	});

	return {
		users: { ...users, list: asArray(users.data) },
		invitations: { ...invitations, list: asArray(invitations.data) },
		requests: { ...requests, list: asArray(requests.data) },
		invite,
		withdraw,
		setRole,
		transfer,
		remove,
		leave,
		answerRequest,
	};
}

export type PackageAccessModel = ReturnType<typeof usePackageAccess>;
