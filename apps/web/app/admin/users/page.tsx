"use client";

import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	GlobalPermission,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Switch,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@tm9657/flow-like-ui";
import { useDebounce } from "@uidotdev/usehooks";
import { formatDistanceToNow } from "date-fns";
import {
	AlertTriangle,
	CheckCircle,
	RefreshCw,
	Search,
	Settings2,
	Shield,
	UserCheck,
	UserX,
	Users,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

type UserStatus = "ACTIVE" | "INACTIVE" | "BANNED";
type UserTier = "FREE" | "PREMIUM" | "PRO" | "ENTERPRISE";

interface AdminUserRecord {
	id: string;
	email?: string;
	username?: string;
	preferred_username?: string;
	name?: string;
	avatar?: string;
	status: UserStatus;
	tier: UserTier;
	permission: number;
	total_size: number;
	total_llm_price: number;
	total_embedding_price: number;
	created_at: string;
	updated_at: string;
}

interface ListUsersResponse {
	users: AdminUserRecord[];
	total: number;
	offset: number;
	limit: number;
}

const STATUS_VARIANTS: Record<
	UserStatus,
	"default" | "secondary" | "destructive" | "outline"
> = {
	ACTIVE: "default",
	INACTIVE: "secondary",
	BANNED: "destructive",
};

const ALL_PERMISSIONS: { label: string; perm: GlobalPermission }[] = [
	{ label: "Admin", perm: GlobalPermission.Admin },
	{ label: "Read Publishing", perm: GlobalPermission.ReadPublishing },
	{ label: "Write Publishing", perm: GlobalPermission.WritePublishing },
	{ label: "Read Profile", perm: GlobalPermission.ReadProfile },
	{ label: "Write Profile", perm: GlobalPermission.WriteProfile },
	{ label: "Read Apps", perm: GlobalPermission.ReadApps },
	{ label: "Write Apps", perm: GlobalPermission.WriteApps },
	{ label: "Write Landing Page", perm: GlobalPermission.WriteLandingPage },
	{ label: "Read Transactions", perm: GlobalPermission.ReadTransactions },
	{ label: "Write Transactions", perm: GlobalPermission.WriteTransactions },
	{ label: "Write Bits", perm: GlobalPermission.WriteBits },
	{ label: "Read Solutions", perm: GlobalPermission.ReadSolutions },
	{ label: "Write Solutions", perm: GlobalPermission.WriteSolutions },
	{ label: "Manage Packages", perm: GlobalPermission.ManagePackages },
	{ label: "Read Courses", perm: GlobalPermission.ReadCourses },
	{ label: "Write Courses", perm: GlobalPermission.WriteCourses },
];

const SKELETON_ROWS = [
	"user-skeleton-1",
	"user-skeleton-2",
	"user-skeleton-3",
	"user-skeleton-4",
	"user-skeleton-5",
	"user-skeleton-6",
	"user-skeleton-7",
	"user-skeleton-8",
] as const;

function formatBytes(bytes: number) {
	if (bytes === 0) return "0 B";
	const k = 1024;
	const sizes = ["B", "KB", "MB", "GB"];
	const i = Math.floor(Math.log(bytes) / Math.log(k));
	return `${(bytes / k ** i).toFixed(1)} ${sizes[i]}`;
}

function formatCents(cents: number) {
	return `$${(cents / 100).toFixed(2)}`;
}

function PermissionDialog({
	user,
	onUpdate,
}: {
	user: AdminUserRecord;
	onUpdate: (userId: string, permission: number) => Promise<void>;
}) {
	const [open, setOpen] = useState(false);
	const [permBits, setPermBits] = useState(user.permission);
	const [saving, setSaving] = useState(false);

	const currentPerm = useMemo(() => new GlobalPermission(permBits), [permBits]);

	const toggle = (perm: GlobalPermission) => {
		const updated = currentPerm.contains(perm)
			? currentPerm.remove(perm)
			: currentPerm.insert(perm);
		setPermBits(updated.toNumber());
	};

	const handleSave = async () => {
		setSaving(true);
		try {
			await onUpdate(user.id, permBits);
			setOpen(false);
		} finally {
			setSaving(false);
		}
	};

	return (
		<>
			<Button variant="outline" size="sm" onClick={() => setOpen(true)}>
				<Settings2 className="h-3 w-3" />
			</Button>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="max-w-md">
					<DialogHeader>
						<DialogTitle>Edit Permissions</DialogTitle>
						<DialogDescription>
							{user.name ?? user.preferred_username ?? user.email ?? user.id}
						</DialogDescription>
					</DialogHeader>
					<div className="grid grid-cols-2 gap-3 py-2">
						{ALL_PERMISSIONS.map(({ label, perm }) => (
							<div key={label} className="flex items-center gap-2">
								<Switch
									id={`perm-${label}`}
									checked={currentPerm.contains(perm)}
									onCheckedChange={() => toggle(perm)}
								/>
								<Label htmlFor={`perm-${label}`} className="text-sm">
									{label}
								</Label>
							</div>
						))}
					</div>
					<DialogFooter>
						<Button variant="outline" onClick={() => setOpen(false)}>
							Cancel
						</Button>
						<Button onClick={handleSave} disabled={saving}>
							{saving ? "Saving…" : "Save"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</>
	);
}

function UserRow({
	user,
	onUpdateStatus,
	onUpdateTier,
	onUpdatePermission,
}: {
	user: AdminUserRecord;
	onUpdateStatus: (userId: string, status: UserStatus) => Promise<void>;
	onUpdateTier: (userId: string, tier: UserTier) => Promise<void>;
	onUpdatePermission: (userId: string, permission: number) => Promise<void>;
}) {
	const [statusLoading, setStatusLoading] = useState(false);

	const displayName =
		user.name ?? user.preferred_username ?? user.username ?? "—";
	const initials = displayName.slice(0, 2).toUpperCase();
	const relativeDate = user.created_at
		? formatDistanceToNow(new Date(user.created_at), { addSuffix: true })
		: "—";

	const handleToggleBan = async () => {
		setStatusLoading(true);
		try {
			await onUpdateStatus(
				user.id,
				user.status === "BANNED" ? "ACTIVE" : "BANNED",
			);
		} finally {
			setStatusLoading(false);
		}
	};

	return (
		<TableRow>
			<TableCell>
				<div className="flex items-center gap-3">
					<Avatar className="h-8 w-8">
						<AvatarImage src={user.avatar ?? ""} />
						<AvatarFallback className="text-xs">{initials}</AvatarFallback>
					</Avatar>
					<div className="min-w-0">
						<p className="text-sm font-medium truncate max-w-[160px]">
							{displayName}
						</p>
						<p className="text-xs text-muted-foreground truncate max-w-[160px]">
							{user.email ?? user.id}
						</p>
					</div>
				</div>
			</TableCell>
			<TableCell>
				<Badge variant={STATUS_VARIANTS[user.status]}>
					{user.status === "ACTIVE" && <CheckCircle className="h-3 w-3 mr-1" />}
					{user.status === "BANNED" && <UserX className="h-3 w-3 mr-1" />}
					{user.status === "INACTIVE" && (
						<AlertTriangle className="h-3 w-3 mr-1" />
					)}
					{user.status.toLowerCase()}
				</Badge>
			</TableCell>
			<TableCell>
				<Select
					value={user.tier}
					onValueChange={(v) => onUpdateTier(user.id, v as UserTier)}
				>
					<SelectTrigger className="h-7 w-[110px] text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="FREE">Free</SelectItem>
						<SelectItem value="PREMIUM">Premium</SelectItem>
						<SelectItem value="PRO">Pro</SelectItem>
						<SelectItem value="ENTERPRISE">Enterprise</SelectItem>
					</SelectContent>
				</Select>
			</TableCell>
			<TableCell className="text-xs text-muted-foreground">
				{formatBytes(user.total_size)}
			</TableCell>
			<TableCell className="text-xs text-muted-foreground">
				{formatCents(user.total_llm_price)}
			</TableCell>
			<TableCell className="text-xs text-muted-foreground">
				{relativeDate}
			</TableCell>
			<TableCell>
				<div className="flex items-center gap-1">
					<Button
						variant={user.status === "BANNED" ? "default" : "destructive"}
						size="sm"
						disabled={statusLoading}
						onClick={handleToggleBan}
					>
						{user.status === "BANNED" ? (
							<UserCheck className="h-3 w-3" />
						) : (
							<UserX className="h-3 w-3" />
						)}
					</Button>
					<PermissionDialog user={user} onUpdate={onUpdatePermission} />
				</div>
			</TableCell>
		</TableRow>
	);
}

export default function AdminUsersPage() {
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [page, setPage] = useState(1);
	const limit = 25;
	const [statusFilter, setStatusFilter] = useState<UserStatus | "all">("all");
	const [tierFilter, setTierFilter] = useState<UserTier | "all">("all");
	const [searchQuery, setSearchQuery] = useState("");
	const debouncedSearch = useDebounce(searchQuery, 300);

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			offset: (page - 1) * limit,
			limit,
		};
		if (statusFilter !== "all") params.status = statusFilter;
		if (tierFilter !== "all") params.tier = tierFilter;
		if (debouncedSearch) params.query = debouncedSearch;
		return params;
	}, [page, statusFilter, tierFilter, debouncedSearch]);

	const users = useQuery<ListUsersResponse>({
		queryKey: ["admin", "users", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<ListUsersResponse>(
				profile.data,
				`admin/users?${qs}`,
			);
		},
		enabled: !!profile.data,
	});

	const totalPages = Math.ceil((users.data?.total ?? 0) / limit);

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
	}, [queryClient]);

	const patchUser = useCallback(
		async (userId: string, body: Record<string, unknown>) => {
			if (!profile.data) throw new Error("Profile not loaded");
			await backend.apiState.patch(profile.data, `admin/users/${userId}`, body);
			queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
		},
		[profile.data, backend.apiState, queryClient],
	);

	const handleUpdateStatus = useCallback(
		async (userId: string, status: UserStatus) => {
			try {
				await patchUser(userId, { status });
				toast.success(`User ${status === "BANNED" ? "banned" : "unbanned"}`);
			} catch {
				toast.error("Failed to update user status");
			}
		},
		[patchUser],
	);

	const handleUpdateTier = useCallback(
		async (userId: string, tier: UserTier) => {
			try {
				await patchUser(userId, { tier });
				toast.success(`Tier updated to ${tier.toLowerCase()}`);
			} catch {
				toast.error("Failed to update tier");
			}
		},
		[patchUser],
	);

	const handleUpdatePermission = useCallback(
		async (userId: string, permission: number) => {
			try {
				await patchUser(userId, { permission });
				toast.success("Permissions updated");
			} catch {
				toast.error("Failed to update permissions");
			}
		},
		[patchUser],
	);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-6xl space-y-6">
					<div className="flex items-center justify-between">
						<div>
							<h1 className="text-3xl font-bold flex items-center gap-2">
								<Users className="h-7 w-7" />
								User Management
							</h1>
							<p className="text-muted-foreground">
								Manage user accounts, tiers, and permissions
							</p>
						</div>
						<Button onClick={handleRefresh} variant="outline" size="sm">
							<RefreshCw className="h-4 w-4 mr-2" />
							Refresh
						</Button>
					</div>

					<div className="grid gap-4 sm:grid-cols-3">
						<Card>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">
									Total Users
								</CardTitle>
								<Users className="h-4 w-4 text-muted-foreground" />
							</CardHeader>
							<CardContent>
								{users.isLoading ? (
									<Skeleton className="h-8 w-16" />
								) : (
									<div className="text-2xl font-bold">
										{users.data?.total ?? 0}
									</div>
								)}
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">
									Current Page
								</CardTitle>
								<Shield className="h-4 w-4 text-muted-foreground" />
							</CardHeader>
							<CardContent>
								<div className="text-2xl font-bold">
									{page} / {Math.max(1, totalPages)}
								</div>
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">Showing</CardTitle>
								<CheckCircle className="h-4 w-4 text-muted-foreground" />
							</CardHeader>
							<CardContent>
								<div className="text-2xl font-bold">
									{users.data?.users.length ?? 0}
								</div>
							</CardContent>
						</Card>
					</div>

					<Card>
						<CardHeader>
							<CardTitle>Users</CardTitle>
							<CardDescription>
								{users.data?.total ?? 0} total users
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="flex flex-wrap items-center gap-3 mb-4">
								<div className="relative flex-1 min-w-[200px] max-w-sm">
									<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
									<Input
										placeholder="Search by name, email, username…"
										value={searchQuery}
										onChange={(e) => {
											setSearchQuery(e.target.value);
											setPage(1);
										}}
										className="pl-10"
									/>
								</div>
								<Select
									value={statusFilter}
									onValueChange={(v) => {
										setStatusFilter(v as UserStatus | "all");
										setPage(1);
									}}
								>
									<SelectTrigger className="w-40">
										<SelectValue placeholder="Status" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="all">All statuses</SelectItem>
										<SelectItem value="ACTIVE">Active</SelectItem>
										<SelectItem value="INACTIVE">Inactive</SelectItem>
										<SelectItem value="BANNED">Banned</SelectItem>
									</SelectContent>
								</Select>
								<Select
									value={tierFilter}
									onValueChange={(v) => {
										setTierFilter(v as UserTier | "all");
										setPage(1);
									}}
								>
									<SelectTrigger className="w-36">
										<SelectValue placeholder="Tier" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="all">All tiers</SelectItem>
										<SelectItem value="FREE">Free</SelectItem>
										<SelectItem value="PREMIUM">Premium</SelectItem>
										<SelectItem value="PRO">Pro</SelectItem>
										<SelectItem value="ENTERPRISE">Enterprise</SelectItem>
									</SelectContent>
								</Select>
							</div>

							{users.isLoading ? (
								<div className="space-y-2">
									{SKELETON_ROWS.map((key) => (
										<Skeleton key={key} className="h-12 w-full" />
									))}
								</div>
							) : (
								<Table>
									<TableHeader>
										<TableRow>
											<TableHead>User</TableHead>
											<TableHead>Status</TableHead>
											<TableHead>Tier</TableHead>
											<TableHead>Storage</TableHead>
											<TableHead>LLM Spend</TableHead>
											<TableHead>Joined</TableHead>
											<TableHead>Actions</TableHead>
										</TableRow>
									</TableHeader>
									<TableBody>
										{users.data?.users.map((u) => (
											<UserRow
												key={u.id}
												user={u}
												onUpdateStatus={handleUpdateStatus}
												onUpdateTier={handleUpdateTier}
												onUpdatePermission={handleUpdatePermission}
											/>
										))}
										{(users.data?.users.length ?? 0) === 0 && (
											<TableRow>
												<TableCell
													colSpan={7}
													className="text-center py-8 text-muted-foreground"
												>
													No users found
												</TableCell>
											</TableRow>
										)}
									</TableBody>
								</Table>
							)}

							{totalPages > 1 && (
								<div className="flex items-center justify-between mt-4">
									<div className="text-sm text-muted-foreground">
										Page {page} of {totalPages}
									</div>
									<div className="flex gap-2">
										<Button
											variant="outline"
											size="sm"
											onClick={() => setPage((p) => Math.max(1, p - 1))}
											disabled={page === 1}
										>
											Previous
										</Button>
										<Button
											variant="outline"
											size="sm"
											onClick={() =>
												setPage((p) => Math.min(totalPages, p + 1))
											}
											disabled={page === totalPages}
										>
											Next
										</Button>
									</div>
								</div>
							)}
						</CardContent>
					</Card>
				</div>
			</div>
		</main>
	);
}
