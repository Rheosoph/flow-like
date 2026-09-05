"use client";

import { useTranslation } from "@flow-like/locales";
import { Eye, EyeOff, Loader2 } from "lucide-react";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { Alert, AlertDescription } from "../ui/alert";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { accountError } from "./account-model";

export interface ChangePasswordDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onPasswordChange: (
		currentPassword: string,
		newPassword: string,
	) => Promise<void>;
}

export default function ChangePasswordDialog({
	open,
	onOpenChange,
	onPasswordChange,
}: ChangePasswordDialogProps) {
	const { t } = useTranslation("common");
	const [values, setValues] = useState({ current: "", next: "", confirm: "" });
	const [visible, setVisible] = useState({
		current: false,
		next: false,
		confirm: false,
	});
	const [pending, setPending] = useState(false);
	const pendingRef = useRef(false);
	const [error, setError] = useState("");
	const close = () => {
		if (pendingRef.current) return;
		setValues({ current: "", next: "", confirm: "" });
		setVisible({ current: false, next: false, confirm: false });
		setError("");
		onOpenChange(false);
	};

	async function submit(event: React.FormEvent) {
		event.preventDefault();
		if (pendingRef.current) return;
		if (!values.current || !values.next) {
			setError(
				t("accountPasswordRequired", "Enter your current and new passwords."),
			);
			return;
		}
		if (values.next !== values.confirm) {
			setError(t("accountPasswordMismatch", "The new passwords do not match."));
			return;
		}
		if (values.current === values.next) {
			setError(
				t(
					"accountPasswordDifferent",
					"Choose a password different from your current password.",
				),
			);
			return;
		}
		pendingRef.current = true;
		setPending(true);
		setError("");
		try {
			await onPasswordChange(values.current, values.next);
			pendingRef.current = false;
			close();
			toast.success(t("accountPasswordUpdated", "Password updated."));
		} catch (cause) {
			setError(
				accountError(
					cause,
					t(
						"accountPasswordFailed",
						"Your password could not be changed. Try again.",
					),
				),
			);
		} finally {
			pendingRef.current = false;
			setPending(false);
		}
	}

	const fields = [
		{
			key: "current",
			label: t("currentPassword", "Current password"),
			autocomplete: "current-password",
		},
		{
			key: "next",
			label: t("newPassword", "New password"),
			autocomplete: "new-password",
		},
		{
			key: "confirm",
			label: t("confirmNewPassword", "Confirm new password"),
			autocomplete: "new-password",
		},
	] as const;

	return (
		<Dialog
			open={open}
			onOpenChange={(value) => {
				if (!value) close();
			}}
		>
			<DialogContent className="sm:max-w-md" showCloseButton={!pending}>
				<DialogHeader>
					<DialogTitle>{t("changePassword", "Change password")}</DialogTitle>
					<DialogDescription>
						{t(
							"accountPasswordHelp",
							"Use a unique password. Your account's password requirements are checked when you save.",
						)}
					</DialogDescription>
				</DialogHeader>
				<form onSubmit={submit} className="space-y-4" aria-busy={pending}>
					{error && (
						<Alert variant="destructive" role="alert">
							<AlertDescription>{error}</AlertDescription>
						</Alert>
					)}
					{fields.map(({ key, label, autocomplete }) => (
						<div className="space-y-2" key={key}>
							<Label htmlFor={`account-password-${key}`}>{label}</Label>
							<div className="relative">
								<Input
									id={`account-password-${key}`}
									name={key}
									autoComplete={autocomplete}
									type={visible[key] ? "text" : "password"}
									className="pr-12"
									required
									disabled={pending}
									value={values[key]}
									onChange={(event) => {
										setValues((previous) => ({
											...previous,
											[key]: event.target.value,
										}));
										setError("");
									}}
								/>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									className="absolute right-0 top-0 h-full"
									disabled={pending}
									aria-label={
										visible[key]
											? t("accountHidePassword", "Hide {{field}}", {
													field: label.toLowerCase(),
												})
											: t("accountShowPassword", "Show {{field}}", {
													field: label.toLowerCase(),
												})
									}
									aria-pressed={visible[key]}
									onClick={() =>
										setVisible((previous) => ({
											...previous,
											[key]: !previous[key],
										}))
									}
								>
									{visible[key] ? (
										<EyeOff className="size-4" />
									) : (
										<Eye className="size-4" />
									)}
								</Button>
							</div>
						</div>
					))}
					<div className="flex flex-col-reverse gap-2 pt-2 sm:flex-row sm:justify-end">
						<Button
							type="button"
							variant="outline"
							onClick={close}
							disabled={pending}
						>
							{t("cancel", "Cancel")}
						</Button>
						<Button type="submit" disabled={pending}>
							{pending && <Loader2 className="size-4 animate-spin" />}
							{pending
								? t("saving", "Saving...")
								: t("changePassword", "Change password")}
						</Button>
					</div>
				</form>
			</DialogContent>
		</Dialog>
	);
}
