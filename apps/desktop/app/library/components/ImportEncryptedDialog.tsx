"use client";

import {
	Button,
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	type IApp,
	IAppVisibility,
	Input,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { EyeIcon, EyeOffIcon, LockIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { appsDB } from "../../../lib/apps-db";

export interface ImportEncryptedDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	path: string | null;
	onImported: () => Promise<void> | void;
}

const ImportEncryptedDialog: React.FC<ImportEncryptedDialogProps> = ({
	open,
	onOpenChange,
	path,
	onImported,
}) => {
	const { t } = useTranslation("common");
	const [password, setPassword] = useState("");
	const [show, setShow] = useState(false);
	const [loading, setLoading] = useState(false);

	useEffect(() => {
		if (!open) {
			setPassword("");
			setShow(false);
			setLoading(false);
		}
	}, [open]);

	const handleImport = useCallback(async () => {
		if (!path) return;
		setLoading(true);
		const toastId = toast.loading(
			t("importingEncryptedApp", "Importing encrypted app..."),
			{
				description: t(
					"decryptingAndImportingPleaseWait",
					"Decrypting and importing. Please wait.",
				),
			},
		);
		try {
			const app = await invoke<IApp>("import_app_from_file", {
				path,
				password,
			});
			await appsDB.visibility.put({
				visibility: app.visibility ?? IAppVisibility.Offline,
				appId: app.id,
			});
			toast.success(
				t("appImportedSuccessfully", "App imported successfully!"),
				{ id: toastId },
			);
			onOpenChange(false);
			await onImported();
		} catch (err) {
			console.error(err);
			toast.error(`Failed to import app`, { id: toastId });
		} finally {
			setLoading(false);
		}
	}, [path, password, onImported, onOpenChange]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md animate-in fade-in-0 slide-in-from-top-8 rounded-2xl shadow-2xl border-none bg-background/95 backdrop-blur-lg max-w-screen overflow-hidden">
				<DialogHeader className="space-y-3">
					<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
						<LockIcon className="h-6 w-6 text-primary" />
					</div>
					<DialogTitle className="text-center text-2xl font-bold">
						{t("importEncryptedApp", "Import Encrypted App")}
					</DialogTitle>
					<DialogDescription className="text-center text-muted-foreground">
						{`This file is encrypted. Enter the password to decrypt and import it.`}
					</DialogDescription>
				</DialogHeader>

				<div className="flex flex-col gap-3 py-2">
					<div className="grid gap-2">
						<label
							htmlFor="import-password"
							className="text-xs text-muted-foreground"
						>
							{t("password", "Password")}
						</label>
						<div className="relative">
							<Input
								id="import-password"
								type={show ? "text" : "password"}
								value={password}
								onChange={(e) => setPassword(e.target.value)}
								placeholder={t("enterPassword", "Enter password")}
								autoFocus
							/>
							<Button
								type="button"
								variant="ghost"
								size="icon"
								className="absolute right-1 top-1 h-7 w-7"
								onClick={() => setShow((s) => !s)}
								aria-label={show ? "Hide password" : "Show password"}
							>
								{show ? (
									<EyeOffIcon className="w-4 h-4" />
								) : (
									<EyeIcon className="w-4 h-4" />
								)}
							</Button>
						</div>
					</div>
				</div>

				<DialogFooter className="flex flex-row gap-1 justify-center pt-2">
					<DialogClose asChild>
						<Button variant="outline" disabled={loading}>
							{t("cancel", "Cancel")}
						</Button>
					</DialogClose>
					<Button
						onClick={handleImport}
						disabled={loading || password.trim() === ""}
					>
						{loading ? "Importing..." : "Import"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};

export default ImportEncryptedDialog;
