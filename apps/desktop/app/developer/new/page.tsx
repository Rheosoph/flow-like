"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Badge, Button, Input, Label, cn } from "@tm9657/flow-like-ui";
import type {
	DeveloperProject,
	TemplateLanguage,
} from "@tm9657/flow-like-ui/lib/schema/developer";
import { TEMPLATE_LANGUAGES } from "@tm9657/flow-like-ui/lib/schema/developer";
import { AnimatePresence, motion } from "framer-motion";
import {
	ArrowLeft,
	Check,
	ChevronRight,
	FolderOpen,
	Loader2,
	Rocket,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { toast } from "sonner";

type WizardStep = "language" | "details" | "creating";

const STEPS: WizardStep[] = ["language", "details", "creating"];

function StepDots({ currentStep }: { currentStep: WizardStep }) {
	const currentIdx = STEPS.indexOf(currentStep);

	return (
		<div className="flex items-center gap-1.5">
			{STEPS.map((step, idx) => {
				const isCompleted = idx < currentIdx;
				const isCurrent = step === currentStep;

				return (
					<div key={step} className="flex items-center gap-1.5">
						<div
							className={cn(
								"rounded-full transition-all duration-300",
								isCompleted
									? "w-2 h-2 bg-primary"
									: isCurrent
										? "w-3 h-3 bg-primary"
										: "w-2 h-2 bg-muted-foreground/20",
							)}
						/>
						{idx < STEPS.length - 1 && (
							<div
								className={cn(
									"w-6 h-px transition-colors",
									idx < currentIdx ? "bg-primary" : "bg-muted-foreground/15",
								)}
							/>
						)}
					</div>
				);
			})}
		</div>
	);
}

function LanguageTile({
	language,
	selected,
	onSelect,
}: {
	language: (typeof TEMPLATE_LANGUAGES)[number];
	selected: boolean;
	onSelect: () => void;
}) {
	return (
		<button
			type="button"
			onClick={onSelect}
			className={cn(
				"relative text-left transition-all duration-200 p-4",
				selected
					? "rounded-xl border border-primary/40 bg-primary/5 ring-1 ring-primary/20"
					: "rounded-xl border border-border/20 bg-card/50 hover:bg-muted/10 hover:border-border/40",
			)}
		>
			{selected && (
				<motion.div
					className="absolute top-2.5 right-2.5"
					initial={{ scale: 0, opacity: 0 }}
					animate={{ scale: 1, opacity: 1 }}
					exit={{ scale: 0, opacity: 0 }}
					transition={{ type: "spring", stiffness: 500, damping: 30 }}
				>
					<Badge
						variant="default"
						className="h-5 w-5 p-0 flex items-center justify-center rounded-full"
					>
						<Check className="h-3 w-3" />
					</Badge>
				</motion.div>
			)}
			<img
				src={language.img}
				alt={language.label}
				className="w-8 h-8 rounded object-cover mb-2"
			/>
			<span className="text-sm font-medium block">{language.label}</span>
			<span className="text-xs text-muted-foreground/70 line-clamp-2 mt-0.5">
				{language.description}
			</span>
		</button>
	);
}

export default function NewProjectWizard() {
	const router = useRouter();
	const [step, setStep] = useState<WizardStep>("language");
	const [language, setLanguage] = useState<TemplateLanguage | null>(null);
	const [projectName, setProjectName] = useState("");
	const [targetDir, setTargetDir] = useState("");
	const [isCreating, setIsCreating] = useState(false);

	const selectDirectory = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (selected) setTargetDir(selected);
	}, []);

	const handleCreate = useCallback(async () => {
		if (!language || !projectName || !targetDir) return;
		setStep("creating");
		setIsCreating(true);
		try {
			const project = await invoke<DeveloperProject>(
				"developer_scaffold_project",
				{
					input: {
						targetDir: `${targetDir}/${projectName.toLowerCase().replace(/\s+/g, "-")}`,
						language,
						projectName,
					},
				},
			);
			toast.success(`Project "${project.name}" created!`);
			router.push("/store/packages?tab=projects");
		} catch (err) {
			toast.error(`Failed to create project: ${err}`);
			setStep("details");
		} finally {
			setIsCreating(false);
		}
	}, [language, projectName, targetDir, router]);

	const selectedLanguageInfo = language
		? TEMPLATE_LANGUAGES.find((l) => l.value === language)
		: null;

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between py-6">
				<div className="flex items-center gap-4">
					<button
						type="button"
						onClick={() =>
							step === "details"
								? setStep("language")
								: router.push("/store/packages?tab=projects")
						}
						className="h-8 w-8 rounded-full flex items-center justify-center text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 transition-colors"
					>
						<ArrowLeft className="h-4 w-4" />
					</button>
					<div>
						<h1 className="text-2xl font-semibold tracking-tight">
							New Node Project
						</h1>
						<p className="text-sm text-muted-foreground/70">
							Scaffold a WASM node from a template
						</p>
					</div>
				</div>
				<StepDots currentStep={step} />
			</div>

			<div className="flex-1 overflow-y-auto">
				<div className="max-w-2xl mx-auto w-full pb-12">
					<AnimatePresence mode="wait">
						{step === "language" && (
							<motion.div
								key="language"
								initial={{ opacity: 0, y: 8 }}
								animate={{ opacity: 1, y: 0 }}
								exit={{ opacity: 0, y: -8 }}
								transition={{ duration: 0.15 }}
								className="space-y-6"
							>
								<div>
									<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60 mb-1">
										Step 1
									</p>
									<h2 className="text-lg font-medium">Choose a language</h2>
									<p className="text-sm text-muted-foreground/70 mt-1">
										Select the programming language for your node project.
									</p>
								</div>

								<div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
									{TEMPLATE_LANGUAGES.map((lang) => (
										<LanguageTile
											key={lang.value}
											language={lang}
											selected={language === lang.value}
											onSelect={() => setLanguage(lang.value)}
										/>
									))}
								</div>

								<div className="flex justify-end pt-2">
									<Button
										onClick={() => setStep("details")}
										disabled={!language}
										className="gap-1.5"
									>
										Continue
										<ChevronRight className="h-4 w-4" />
									</Button>
								</div>
							</motion.div>
						)}

						{step === "details" && (
							<motion.div
								key="details"
								initial={{ opacity: 0, y: 8 }}
								animate={{ opacity: 1, y: 0 }}
								exit={{ opacity: 0, y: -8 }}
								transition={{ duration: 0.15 }}
								className="space-y-6"
							>
								<div>
									<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60 mb-1">
										Step 2
									</p>
									<h2 className="text-lg font-medium">Project details</h2>
									<p className="text-sm text-muted-foreground/70 mt-1">
										Configure your new {selectedLanguageInfo?.label ?? ""}{" "}
										project.
									</p>
								</div>

								{selectedLanguageInfo && (
									<div className="flex items-center gap-3 rounded-xl border border-border/20 bg-muted/5 p-4">
										<img
											src={selectedLanguageInfo.img}
											alt={selectedLanguageInfo.label}
											className="w-8 h-8 rounded object-cover"
										/>
										<div>
											<p className="text-sm font-medium">
												{selectedLanguageInfo.label}
											</p>
											<p className="text-xs text-muted-foreground/70">
												{selectedLanguageInfo.description}
											</p>
										</div>
									</div>
								)}

								<div className="space-y-4">
									<div className="space-y-2">
										<Label
											htmlFor="name"
											className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60"
										>
											Project Name
										</Label>
										<Input
											id="name"
											placeholder="my-custom-node"
											value={projectName}
											onChange={(e) => setProjectName(e.target.value)}
											className="h-10 rounded-lg bg-muted/5"
										/>
									</div>

									<div className="space-y-2">
										<Label className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
											Target Directory
										</Label>
										<div className="flex gap-2">
											<Input
												value={targetDir}
												readOnly
												placeholder="Select a directory…"
												className="flex-1 h-10 rounded-lg bg-muted/5"
											/>
											<Button
												variant="outline"
												size="icon"
												onClick={selectDirectory}
												className="h-10 w-10 shrink-0 rounded-lg"
											>
												<FolderOpen className="h-4 w-4" />
											</Button>
										</div>
										{targetDir && projectName && (
											<motion.p
												initial={{ opacity: 0, height: 0 }}
												animate={{
													opacity: 1,
													height: "auto",
												}}
												className="text-xs text-muted-foreground/60 px-1 pt-1"
											>
												→{" "}
												<code className="text-primary/80 font-mono">
													{targetDir}/
													{projectName.toLowerCase().replace(/\s+/g, "-")}
												</code>
											</motion.p>
										)}
									</div>
								</div>

								<div className="flex justify-between pt-4">
									<Button
										variant="ghost"
										onClick={() => setStep("language")}
										className="gap-1.5 text-muted-foreground/60 hover:text-foreground/80"
									>
										<ArrowLeft className="h-4 w-4" />
										Back
									</Button>
									<Button
										onClick={handleCreate}
										disabled={!projectName || !targetDir || isCreating}
										className="gap-1.5"
									>
										<Rocket className="h-4 w-4" />
										Create Project
									</Button>
								</div>
							</motion.div>
						)}

						{step === "creating" && (
							<motion.div
								key="creating"
								initial={{ opacity: 0 }}
								animate={{ opacity: 1 }}
								className="flex flex-col items-center justify-center py-24 text-center"
							>
								<Loader2 className="h-8 w-8 animate-spin text-primary mb-6" />
								<h2 className="text-lg font-medium mb-1">
									Creating your project…
								</h2>
								<p className="text-sm text-muted-foreground/70 max-w-sm">
									Downloading the template and scaffolding your project. This
									may take a moment.
								</p>
							</motion.div>
						)}
					</AnimatePresence>
				</div>
			</div>
		</div>
	);
}
