"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy, Mail, User as UserIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
} from "../../../../lib/user-display";
import { useBackend } from "../../../../state/backend-state";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	RelativeTime,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
} from "../../../ui";

interface UserPillProps {
	userId?: string | null;
	className?: string;
	compact?: boolean;
	muted?: boolean;
}

export function UserPill({
	userId,
	className,
	compact = false,
	muted = false,
}: Readonly<UserPillProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const [open, setOpen] = useState(false);

	const lookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		userId ? [userId] : ["__noop__"],
		Boolean(userId),
	);

	const label = userDisplayName(
		lookup.data,
		lookup.data?.id ?? userId ?? "Unknown",
	);
	const initials = useMemo(() => userInitials(label, "??"), [label]);
	const isAnonymous = !userId;

	if (isAnonymous) {
		return (
			<Badge
				variant="outline"
				className={`gap-1 border-dashed text-muted-foreground ${className ?? ""}`}
			>
				<UserIcon className="h-3 w-3" />
				{t('anonymous', 'Anonymous')}
			</Badge>
		);
	}

	const trigger = (
		<button
			type="button"
			onClick={() => setOpen(true)}
			className={`group inline-flex max-w-full items-center gap-1.5 rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 text-xs transition-colors hover:border-primary/50 hover:bg-primary/5 ${
				muted ? "text-muted-foreground" : ""
			} ${className ?? ""}`}
		>
			<Avatar className="h-4 w-4">
				<AvatarImage src={userAvatarUrl(lookup.data) ?? ""} alt={label} />
				<AvatarFallback className="text-[8px]">{initials}</AvatarFallback>
			</Avatar>
			{lookup.isLoading ? (
				<Skeleton className="h-3 w-16" />
			) : (
				<span className="truncate font-medium group-hover:text-primary">
					{compact && label.length > 16 ? `${label.slice(0, 16)}…` : label}
				</span>
			)}
		</button>
	);

	return (
		<>
			{trigger}
			<Sheet open={open} onOpenChange={setOpen}>
				<SheetContent className="w-full sm:max-w-md overflow-y-auto">
					<SheetHeader>
						<div className="flex items-center gap-3">
							<Avatar className="h-12 w-12">
								<AvatarImage
									src={userAvatarUrl(lookup.data) ?? ""}
									alt={label}
								/>
								<AvatarFallback>{initials}</AvatarFallback>
							</Avatar>
							<div className="min-w-0">
								<SheetTitle className="truncate">{label}</SheetTitle>
								<SheetDescription className="truncate font-mono text-xs">
									{userId}
								</SheetDescription>
							</div>
						</div>
					</SheetHeader>
					<div className="space-y-4 px-4 pb-6">
						{lookup.isLoading && (
							<div className="space-y-2">
								<Skeleton className="h-4 w-3/4" />
								<Skeleton className="h-4 w-1/2" />
								<Skeleton className="h-4 w-2/3" />
							</div>
						)}

						{!lookup.isLoading && (
							<div className="grid gap-3 text-sm">
								{lookup.data?.email && (
									<div className="flex items-center justify-between gap-2">
										<span className="text-muted-foreground inline-flex items-center gap-1.5">
											<Mail className="h-3.5 w-3.5" /> {t('email', 'Email')}
										</span>
										<span className="truncate text-right">
											{lookup.data.email}
										</span>
									</div>
								)}
								{lookup.data?.username && (
									<div className="flex items-center justify-between gap-2">
										<span className="text-muted-foreground">{t('username2', 'Username')}</span>
										<span className="truncate text-right">
											{lookup.data.username}
										</span>
									</div>
								)}
								{lookup.data?.preferred_username && (
									<div className="flex items-center justify-between gap-2">
										<span className="text-muted-foreground">{t('preferred', 'Preferred')}</span>
										<span className="truncate text-right">
											{lookup.data.preferred_username}
										</span>
									</div>
								)}
								{lookup.data?.created_at && (
									<div className="flex items-center justify-between gap-2">
										<span className="text-muted-foreground">{t('joined', 'Joined')}</span>
										<RelativeTime
											value={lookup.data.created_at}
											className="text-right"
										/>
									</div>
								)}
								{lookup.data?.description && (
									<>
										<Separator />
										<div className="space-y-1">
											<div className="text-xs uppercase tracking-wide text-muted-foreground">
												{t('description', 'Description')}
											</div>
											<p className="text-sm leading-relaxed">
												{lookup.data.description}
											</p>
										</div>
									</>
								)}
							</div>
						)}

						<Separator />

						<div className="space-y-2">
							<div className="text-xs uppercase tracking-wide text-muted-foreground">
								{t('reference', 'Reference')}
							</div>
							<div className="flex items-center gap-2">
								<code className="flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-xs">
									{userId}
								</code>
								<Button
									size="sm"
									variant="outline"
									onClick={() => {
										if (!userId) return;
										navigator.clipboard.writeText(userId).catch(() => null);
										toast.success("User id copied");
									}}
								>
									<Copy className="h-3.5 w-3.5" />
								</Button>
							</div>
						</div>
					</div>
				</SheetContent>
			</Sheet>
		</>
	);
}
