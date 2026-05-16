"use client";
import {
	ExternalLinkIcon,
	KeyIcon,
	PackageIcon,
	ShieldAlertIcon,
	ShieldCheckIcon,
} from "lucide-react";
import { useCallback, useMemo } from "react";
import { NODE_PERMISSION_LABELS } from "../../lib/permission/node-permission";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { ScrollArea } from "../ui/scroll-area";
import { Separator } from "../ui/separator";

function formatPermission(perm: string): { label: string; icon: string } {
	return (
		NODE_PERMISSION_LABELS[perm as keyof typeof NODE_PERMISSION_LABELS] ?? {
			label: perm,
			icon: "🔒",
		}
	);
}

export type RememberChoice = "none" | "board" | "event" | "package";

export interface WasmSandboxWarningDialogProps {
	open: boolean;
	packageIds: string[];
	packagePermissions?: Record<string, string[]>;
	onConfirm: (rememberFor: RememberChoice) => void;
	onCancel: () => void;
}

export function WasmSandboxWarningDialog({
	open,
	packageIds,
	packagePermissions,
	onConfirm,
	onCancel,
}: WasmSandboxWarningDialogProps) {
	const handleRunOnce = useCallback(() => {
		onConfirm("none");
	}, [onConfirm]);

	const handleAlwaysTrust = useCallback(() => {
		onConfirm("package");
	}, [onConfirm]);

	const handleTrustForBoard = useCallback(() => {
		onConfirm("board");
	}, [onConfirm]);

	const allPermissions = useMemo(() => {
		if (!packagePermissions) return new Map<string, string[]>();
		const result = new Map<string, string[]>();
		for (const pkgId of packageIds) {
			const perms = packagePermissions[pkgId];
			if (perms?.length) result.set(pkgId, perms);
		}
		return result;
	}, [packageIds, packagePermissions]);

	const hasPermissions = allPermissions.size > 0;

	return (
		<Dialog open={open} onOpenChange={(o) => !o && onCancel()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<div className="flex items-center gap-2">
						<ShieldAlertIcon className="w-5 h-5 text-amber-500" />
						<DialogTitle>Sideloaded WASM nodes detected</DialogTitle>
					</div>
					<DialogDescription>
						This workflow contains externally-loaded WebAssembly nodes. They run
						inside an isolated sandbox, but you should only run code you trust.{" "}
						<a
							href="https://docs.flow-like.com/dev/wasm-nodes/sandboxing/"
							target="_blank"
							rel="noopener noreferrer"
							className="inline-flex items-center gap-0.5 text-primary underline underline-offset-2 hover:text-primary/80"
						>
							Learn more
							<ExternalLinkIcon className="w-3 h-3" />
						</a>
					</DialogDescription>
				</DialogHeader>

				<ScrollArea className="max-h-[40vh]">
					<div className="flex flex-col gap-3 py-1 pr-3">
						{packageIds.map((id) => {
							const perms = allPermissions.get(id);
							return (
								<div
									key={id}
									className="flex flex-col gap-1.5 rounded-md border p-2.5"
								>
									<div className="flex items-center gap-1.5">
										<PackageIcon className="w-3.5 h-3.5 text-muted-foreground" />
										<span className="text-sm font-medium">{id}</span>
									</div>
									{perms && perms.length > 0 ? (
										<div className="flex flex-wrap gap-1">
											{perms.map((p) => {
												const { label, icon } = formatPermission(p);
												return (
													<Badge
														key={p}
														variant="outline"
														className="flex items-center gap-1 text-xs"
													>
														<span>{icon}</span>
														{label}
													</Badge>
												);
											})}
										</div>
									) : (
										<span className="text-xs text-muted-foreground">
											No additional permissions requested
										</span>
									)}
								</div>
							);
						})}
					</div>
				</ScrollArea>

				{hasPermissions && (
					<>
						<Separator />
						<div className="flex items-start gap-2 text-xs text-muted-foreground">
							<KeyIcon className="w-3.5 h-3.5 mt-0.5 shrink-0" />
							<span>
								Permissions are declared by each node and enforced by the
								sandbox at runtime.
							</span>
						</div>
					</>
				)}

				<DialogFooter className="flex-col gap-2 sm:flex-col">
					<div className="flex flex-wrap gap-2 w-full justify-end">
						<Button variant="outline" onClick={onCancel}>
							Cancel
						</Button>
						<Button variant="secondary" onClick={handleTrustForBoard}>
							Trust for this board
						</Button>
						<Button variant="secondary" onClick={handleRunOnce}>
							Run once
						</Button>
						<Button
							onClick={handleAlwaysTrust}
							variant="destructive"
							className="gap-1.5"
						>
							<ShieldCheckIcon className="w-4 h-4" />
							Always trust
						</Button>
					</div>
					<p className="text-xs text-muted-foreground text-right">
						&quot;Always trust&quot; remembers your choice for these packages
						across all boards.
					</p>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
