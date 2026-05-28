"use client";

import {
	Button,
	Calendar,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Popover,
	PopoverContent,
	PopoverTrigger,
	RadioGroup,
	RadioGroupItem,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { format } from "date-fns";
import {
	CalendarIcon,
	CheckIcon,
	CopyIcon,
	KeyRoundIcon,
	PlusIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

interface PAT {
	id: string;
	name: string;
	created_at: string;
	valid_until: string | null;
	permission: number;
}

const permissionLevels = [
	{ value: 1, label: "Read Only", description: "View access only" },
	{ value: 2, label: "Read & Write", description: "View and modify access" },
	{ value: 4, label: "Admin", description: "Full administrative access" },
];

const STORAGE_KEY = "flow-like-selected-pat";

// Helper to clear PAT from storage (should be called on logout)
export function clearStoredPat() {
	if (typeof window !== "undefined") {
		localStorage.removeItem(STORAGE_KEY);
	}
}

interface PatSelectorDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onPatSelected: (pat: string) => void;
	title?: string;
	description?: string;
}

export function PatSelectorDialog({
	open,
	onOpenChange,
	onPatSelected,
	title = "Select or Create Personal Access Token",
	description = "Choose an existing token or create a new one for this event sink.",
}: Readonly<PatSelectorDialogProps>) {
	const backend = useBackend();
	const pats = useInvoke(
		backend.userState.getPATs,
		backend.userState,
		[],
		open,
	);

	const [mode, setMode] = useState<"select" | "create">("select");
	const [selectedPatId, setSelectedPatId] = useState<string>("");
	const [creating, setCreating] = useState(false);
	const [newToken, setNewToken] = useState<string>("");
	const [showTokenDialog, setShowTokenDialog] = useState(false);

	// Create form state
	const [tokenName, setTokenName] = useState("");
	const [expirationDate, setExpirationDate] = useState<Date | undefined>(
		undefined,
	);
	const [selectedPermission, setSelectedPermission] = useState<number>(2);

	// Check if we have a stored PAT when dialog opens
	useEffect(() => {
		if (open && typeof window !== "undefined") {
			const savedPat = localStorage.getItem(STORAGE_KEY);
			if (savedPat) {
				// We have a stored PAT, switch to create mode to inform user
				setMode("create");
			}
		}
	}, [open]);

	const handleSelectExisting = async () => {
		// Check if we have a stored PAT
		const storedPat =
			typeof window !== "undefined" ? localStorage.getItem(STORAGE_KEY) : null;

		if (storedPat) {
			// Use the stored PAT
			onPatSelected(storedPat);
			onOpenChange(false);
			return;
		}

		// No stored PAT - inform user they need to create a new one
		toast.error(
			"No stored token available. Please create a new token to continue.",
		);
		setMode("create");
	};

	const handleCreateToken = async () => {
		if (!tokenName.trim()) {
			toast.error("Please enter a token name");
			return;
		}

		if (!backend) {
			toast.error("Backend not available");
			return;
		}

		try {
			setCreating(true);
			const result = await backend.userState.createPAT(
				tokenName,
				expirationDate,
				selectedPermission,
			);

			setNewToken(result.pat);
			setShowTokenDialog(true);

			// Reset form
			setTokenName("");
			setExpirationDate(undefined);
			setSelectedPermission(2);

			// Reload PATs list
			await pats.refetch();
			toast.success("Token created successfully!");
		} catch (error) {
			toast.error("Failed to create token");
			console.error(error);
		} finally {
			setCreating(false);
		}
	};

	const handleUseNewToken = () => {
		// Save the PAT to localStorage for future use
		if (typeof window !== "undefined") {
			localStorage.setItem(STORAGE_KEY, newToken);
		}

		onPatSelected(newToken);
		setShowTokenDialog(false);
		onOpenChange(false);
		setNewToken("");
	};

	const formatDate = (dateString: string) => {
		return format(new Date(dateString), "MMM dd, yyyy");
	};

	const getPermissionLabel = (permission: number) => {
		return (
			permissionLevels.find((p) => p.value === permission)?.label || "Unknown"
		);
	};

	const isExpired = (validUntil: string | null) => {
		if (!validUntil) return false;
		return new Date(validUntil) < new Date();
	};

	return (
		<>
			<Dialog open={open && !showTokenDialog} onOpenChange={onOpenChange}>
				<DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
					<DialogHeader>
						<DialogTitle className="flex items-center gap-2">
							<KeyRoundIcon className="h-5 w-5" />
							{title}
						</DialogTitle>
						<DialogDescription>{description}</DialogDescription>
					</DialogHeader>

					<div className="space-y-6">
						{typeof window !== "undefined" &&
							localStorage.getItem(STORAGE_KEY) && (
								<div className="p-3 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg">
									<div className="flex items-start gap-2">
										<CheckIcon className="h-5 w-5 text-green-600 dark:text-green-400 mt-0.5" />
										<div className="flex-1">
											<p className="text-sm font-medium text-green-900 dark:text-green-100">
												Stored Token Available
											</p>
											<p className="text-xs text-green-700 dark:text-green-300 mt-1">
												You have a previously created token stored. You can use
												it or create a new one.
											</p>
										</div>
									</div>
								</div>
							)}

						<RadioGroup
							value={mode}
							onValueChange={(v) => setMode(v as "select" | "create")}
						>
							<div className="flex items-center space-x-2">
								<RadioGroupItem value="select" id="select" />
								<Label htmlFor="select" className="cursor-pointer">
									Use Stored Token
								</Label>
							</div>
							<div className="flex items-center space-x-2">
								<RadioGroupItem value="create" id="create" />
								<Label htmlFor="create" className="cursor-pointer">
									Create New Token
								</Label>
							</div>
						</RadioGroup>

						<Separator />

						{mode === "select" ? (
							<div className="space-y-4">
								{typeof window !== "undefined" &&
								!localStorage.getItem(STORAGE_KEY) ? (
									<div className="text-center py-8">
										<KeyRoundIcon className="h-12 w-12 text-muted-foreground mx-auto mb-4" />
										<p className="text-muted-foreground mb-4">
											No stored token available. Create a new one to continue.
										</p>
										<Button onClick={() => setMode("create")} className="gap-2">
											<PlusIcon className="h-4 w-4" />
											Create New Token
										</Button>
									</div>
								) : pats.isLoading ? (
									<div className="text-center py-8 text-muted-foreground">
										Loading tokens...
									</div>
								) : pats.data?.length === 0 ? (
									<div className="text-center py-8">
										<KeyRoundIcon className="h-12 w-12 text-muted-foreground mx-auto mb-4" />
										<p className="text-muted-foreground mb-4">
											No tokens available. Create a new one to continue.
										</p>
										<Button
											onClick={() => setMode("create")}
											variant="outline"
											className="gap-2"
										>
											<PlusIcon className="h-4 w-4" />
											Create Token
										</Button>
									</div>
								) : (
									<>
										<div className="flex justify-end gap-2">
											<Button
												variant="outline"
												onClick={() => onOpenChange(false)}
											>
												Cancel
											</Button>
											<Button onClick={handleSelectExisting}>
												Use Stored Token
											</Button>
										</div>
									</>
								)}
							</div>
						) : (
							<div className="space-y-5">
								<div className="space-y-2">
									<Label htmlFor="token-name" className="text-sm font-medium">
										Token Name *
									</Label>
									<Input
										id="token-name"
										placeholder="e.g., Event Sink Token, Production API"
										value={tokenName}
										onChange={(e) => setTokenName(e.target.value)}
										className="w-full"
									/>
									<p className="text-xs text-muted-foreground">
										Choose a descriptive name to identify this token&apos;s
										purpose.
									</p>
								</div>

								<div className="space-y-2">
									<Label className="text-sm font-medium">
										Permission Level *
									</Label>
									<Select
										value={selectedPermission.toString()}
										onValueChange={(v) =>
											setSelectedPermission(Number.parseInt(v))
										}
									>
										<SelectTrigger className="w-full">
											<SelectValue placeholder="Select permission level" />
										</SelectTrigger>
										<SelectContent>
											{permissionLevels.map((level) => (
												<SelectItem
													key={level.value}
													value={level.value.toString()}
												>
													<div className="flex flex-col">
														<span className="font-medium">{level.label}</span>
														<span className="text-xs text-muted-foreground">
															{level.description}
														</span>
													</div>
												</SelectItem>
											))}
										</SelectContent>
									</Select>
								</div>

								<div className="space-y-2">
									<Label className="text-sm font-medium">
										Expiration Date (Optional)
									</Label>
									<Popover>
										<PopoverTrigger asChild>
											<Button
												variant="outline"
												className={cn(
													"w-full justify-start text-left font-normal",
													!expirationDate && "text-muted-foreground",
												)}
											>
												<CalendarIcon className="mr-2 h-4 w-4" />
												{expirationDate ? (
													format(expirationDate, "PPP")
												) : (
													<span>No expiration</span>
												)}
											</Button>
										</PopoverTrigger>
										<PopoverContent className="w-auto p-0">
											<Calendar
												mode="single"
												selected={expirationDate}
												onSelect={setExpirationDate}
												disabled={(date) => date < new Date()}
												initialFocus
											/>
										</PopoverContent>
									</Popover>
									<p className="text-xs text-muted-foreground">
										Leave blank for a token that never expires.
									</p>
								</div>

								<div className="flex justify-end gap-2">
									<Button variant="outline" onClick={() => onOpenChange(false)}>
										Cancel
									</Button>
									<Button
										onClick={handleCreateToken}
										disabled={!tokenName.trim() || creating}
										className="gap-2"
									>
										{creating ? (
											"Creating..."
										) : (
											<>
												<PlusIcon className="h-4 w-4" />
												Create Token
											</>
										)}
									</Button>
								</div>
							</div>
						)}
					</div>
				</DialogContent>
			</Dialog>

			{/* Token Created Success Dialog */}
			<Dialog open={showTokenDialog} onOpenChange={setShowTokenDialog}>
				<DialogContent className="max-w-2xl">
					<div className="space-y-6">
						<div className="flex items-start gap-4">
							<div className="p-3 rounded-full bg-green-100 dark:bg-green-900/20">
								<CheckIcon className="h-6 w-6 text-green-600 dark:text-green-400" />
							</div>
							<div className="flex-1">
								<h2 className="text-2xl font-bold mb-2">
									Token Created Successfully!
								</h2>
								<p className="text-muted-foreground">
									Your personal access token has been generated. Make sure to
									copy it now as you won&apos;t be able to see it again.
								</p>
							</div>
						</div>

						<Separator />

						<div className="space-y-4">
							<div className="p-4 rounded-lg bg-muted/50 border-2 border-dashed border-primary/30">
								<Label className="text-xs font-medium text-muted-foreground mb-2 block">
									YOUR TOKEN
								</Label>
								<div className="flex items-center gap-2 mb-3">
									<code className="flex-1 p-3 rounded bg-background border font-mono text-sm break-all select-all">
										{newToken}
									</code>
								</div>
								<Button
									variant="secondary"
									size="sm"
									onClick={() => {
										navigator.clipboard.writeText(newToken);
										toast.success("Token copied to clipboard!");
									}}
									className="w-full gap-2"
								>
									<CopyIcon className="h-4 w-4" />
									Copy to Clipboard
								</Button>
							</div>

							<div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4">
								<h3 className="font-semibold mb-2 text-destructive">
									⚠️ Important Security Information
								</h3>
								<ul className="space-y-2 text-sm text-muted-foreground">
									<li className="flex gap-2">
										<span>•</span>
										<span>
											This token will only be shown once. Store it securely.
										</span>
									</li>
									<li className="flex gap-2">
										<span>•</span>
										<span>
											Treat this token like a password - don&apos;t share it or
											commit it to version control
										</span>
									</li>
									<li className="flex gap-2">
										<span>•</span>
										<span>
											If compromised, delete it immediately and create a new one
										</span>
									</li>
								</ul>
							</div>
						</div>

						<Separator />

						<div className="flex justify-end gap-2">
							<Button
								variant="outline"
								onClick={() => setShowTokenDialog(false)}
							>
								Cancel
							</Button>
							<Button onClick={handleUseNewToken} className="gap-2">
								<CheckIcon className="h-4 w-4" />
								Use This Token
							</Button>
						</div>
					</div>
				</DialogContent>
			</Dialog>
		</>
	);
}
