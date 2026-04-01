"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Clock, Users, X } from "lucide-react";
import { toast } from "sonner";
import type { AccessRequest } from "../../lib/schema/wasm";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	RelativeTime,
} from "../ui";
import type { GenericFetcher } from "../pages/store/store-package-detail";

export interface PackageAccessTabProps {
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
}

export function PackageAccessTab({
	packageId,
	fetcher,
	auth,
}: PackageAccessTabProps) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const queryClient = useQueryClient();

	const queryKey = ["access-requests", packageId];

	const { data: requests = [], isLoading } = useQuery<AccessRequest[]>({
		queryKey,
		queryFn: () =>
			fetcher<AccessRequest[]>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/access`,
				{ method: "GET" },
				auth,
			),
		enabled: !!profile.data,
	});

	const accept = useMutation({
		mutationFn: (requestId: string) =>
			fetcher<void>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/access/${requestId}`,
				{ method: "POST" },
				auth,
			),
		onSuccess: () => {
			toast.success("Access request accepted");
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () => toast.error("Failed to accept request"),
	});

	const reject = useMutation({
		mutationFn: (requestId: string) =>
			fetcher<void>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/access/${requestId}`,
				{ method: "DELETE" },
				auth,
			),
		onSuccess: () => {
			toast.success("Access request rejected");
			queryClient.invalidateQueries({ queryKey });
		},
		onError: () => toast.error("Failed to reject request"),
	});

	const isMutating = accept.isPending || reject.isPending;

	if (isLoading) {
		return (
			<Card>
				<CardContent className="flex items-center justify-center py-12">
					<Clock className="mr-2 h-4 w-4 animate-spin text-muted-foreground" />
					<span className="text-sm text-muted-foreground">Loading…</span>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between pb-4">
				<CardTitle className="text-base font-medium">
					Access Requests
				</CardTitle>
				<Badge variant="secondary">{requests.length} pending</Badge>
			</CardHeader>

			<CardContent>
				{requests.length === 0 ? (
					<div className="flex flex-col items-center gap-2 py-12 text-muted-foreground">
						<Users className="h-8 w-8" />
						<p className="text-sm">No pending access requests</p>
					</div>
				) : (
					<div className="divide-y">
						{requests.map((req) => (
							<div
								key={req.id}
								className="flex items-center gap-3 py-3 first:pt-0 last:pb-0"
							>
								<div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium uppercase">
									{req.userId.charAt(0)}
								</div>

								<div className="min-w-0 flex-1">
									<p className="truncate text-sm font-medium">{req.userId}</p>
									{req.comment && (
										<p className="truncate text-xs text-muted-foreground">
											{req.comment}
										</p>
									)}
								</div>

								<RelativeTime
									className="shrink-0 text-xs text-muted-foreground"
									value={req.createdAt}
								/>

								<div className="flex shrink-0 gap-1">
									<Button
										size="icon"
										variant="ghost"
										className="h-8 w-8 text-green-600 hover:bg-green-50 hover:text-green-700"
										disabled={isMutating}
										onClick={() => accept.mutate(req.id)}
									>
										<Check className="h-4 w-4" />
									</Button>
									<Button
										size="icon"
										variant="ghost"
										className="h-8 w-8 text-destructive hover:bg-destructive/10"
										disabled={isMutating}
										onClick={() => reject.mutate(req.id)}
									>
										<X className="h-4 w-4" />
									</Button>
								</div>
							</div>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}
