"use client";

import { Badge, Button, Input, Label, cn } from "@flow-like/flow-like-ui";
import type {
	DeveloperProject,
	TemplateLanguage,
	WidgetFramework,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import {
	TEMPLATE_LANGUAGES,
	WIDGET_FRAMEWORKS,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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

type WizardStep = "capabilities" | "details" | "creating";

const STEPS: WizardStep[] = ["capabilities", "details", "creating"];

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

function SelectedCheck() {
	return (
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
	);
}

function CapabilityTile({
	label,
	description,
	selected,
	onSelect,
	img,
	icon,
}: {
	label: string;
	description: string;
	selected: boolean;
	onSelect: () => void;
	img?: string;
	icon?: string;
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
			{selected && <SelectedCheck />}
			{img ? (
				<img
					src={img}
					alt={label}
					className="w-8 h-8 rounded object-cover mb-2"
				/>
			) : (
				<span className="w-8 h-8 rounded bg-muted/40 flex items-center justify-center text-lg mb-2">
					{icon}
				</span>
			)}
			<span className="text-sm font-medium block">{label}</span>
			<span className="text-xs text-muted-foreground/70 line-clamp-2 mt-0.5">
				{description}
			</span>
		</button>
	);
}

function SectionHeading({
	title,
	hint,
}: {
	title: string;
	hint: string;
}) {
	return (
		<div className="flex items-baseline justify-between gap-2">
			<h3 className="text-sm font-medium">{title}</h3>
			<span className="text-xs text-muted-foreground/60">{hint}</span>
		</div>
	);
}

export default function NewProjectWizard() {
	const router = useRouter();
	const [step, setStep] = useState<WizardStep>("capabilities");
	const [nodeLanguage, setNodeLanguage] = useState<TemplateLanguage | null>(
		null,
	);
	const [widgetFrameworks, setWidgetFrameworks] = useState<WidgetFramework[]>(
		[],
	);
	const [projectName, setProjectName] = useState("");
	const [targetDir, setTargetDir] = useState("");
	const [isCreating, setIsCreating] = useState(false);

	const hasCapability = nodeLanguage !== null || widgetFrameworks.length > 0;

	const toggleFramework = useCallback((framework: WidgetFramework) => {
		setWidgetFrameworks((prev) =>
			prev.includes(framework)
				? prev.filter((f) => f !== framework)
				: [...prev, framework],
		);
	}, []);

	const selectDirectory = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (selected) setTargetDir(selected);
	}, []);

	const handleCreate = useCallback(async () => {
		if (!hasCapability || !projectName || !targetDir) return;
		setStep("creating");
		setIsCreating(true);
		try {
			const project = await invoke<DeveloperProject>(
				"developer_scaffold_project",
				{
					input: {
						targetDir: `${targetDir}/${projectName.toLowerCase().replace(/\s+/g, "-")}`,
						projectName,
						nodeLanguage,
						widgetFrameworks,
					},
				},
			);
			toast.success(`Project "${project.name}" created!`);
			router.push("/developer");
		} catch (err) {
			toast.error(`Failed to create project: ${err}`);
			setStep("details");
		} finally {
			setIsCreating(false);
		}
	}, [
		hasCapability,
		nodeLanguage,
		widgetFrameworks,
		projectName,
		targetDir,
		router,
	]);

	const selectedLanguageInfo = nodeLanguage
		? TEMPLATE_LANGUAGES.find((l) => l.value === nodeLanguage)
		: null;
	const selectedFrameworkInfos = WIDGET_FRAMEWORKS.filter((f) =>
		widgetFrameworks.includes(f.value),
	);

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between py-6">
				<div className="flex items-center gap-4">
					<button
						type="button"
						onClick={() =>
							step === "details"
								? setStep("capabilities")
								: router.push("/developer")
						}
						className="h-8 w-8 rounded-full flex items-center justify-center text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 transition-colors"
					>
						<ArrowLeft className="h-4 w-4" />
					</button>
					<div>
						<h1 className="text-2xl font-semibold tracking-tight">
							New Package Project
						</h1>
						<p className="text-sm text-muted-foreground/70">
							Scaffold a package with WASM nodes and/or widgets
						</p>
					</div>
				</div>
				<StepDots currentStep={step} />
			</div>

			<div className="flex-1 overflow-y-auto">
				<div className="max-w-2xl mx-auto w-full pb-12">
					<AnimatePresence mode="wait">
						{step === "capabilities" && (
							<motion.div
								key="capabilities"
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
									<h2 className="text-lg font-medium">Choose capabilities</h2>
									<p className="text-sm text-muted-foreground/70 mt-1">
										Pick a node language, widget frameworks, or both. At least
										one is required.
									</p>
								</div>

								<div className="space-y-3">
									<SectionHeading
										title="Node runtime"
										hint={
											nodeLanguage
												? "Click again to deselect"
												: "No node — widgets only"
										}
									/>
									<div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
										{TEMPLATE_LANGUAGES.map((lang) => (
											<CapabilityTile
												key={lang.value}
												label={lang.label}
												description={lang.description}
												img={lang.img}
												selected={nodeLanguage === lang.value}
												onSelect={() =>
													setNodeLanguage((prev) =>
														prev === lang.value ? null : lang.value,
													)
												}
											/>
										))}
									</div>
								</div>

								<div className="space-y-3">
									<SectionHeading title="Widgets" hint="Select any number" />
									<div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
										{WIDGET_FRAMEWORKS.map((framework) => (
											<CapabilityTile
												key={framework.value}
												label={framework.label}
												description={framework.description}
												icon={framework.icon}
												selected={widgetFrameworks.includes(framework.value)}
												onSelect={() => toggleFramework(framework.value)}
											/>
										))}
									</div>
								</div>

								<div className="flex justify-end pt-2">
									<Button
										onClick={() => setStep("details")}
										disabled={!hasCapability}
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
										Name your project and pick where it lives.
									</p>
								</div>

								<div className="rounded-xl border border-border/20 bg-muted/5 p-4 space-y-2">
									<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
										Will be scaffolded
									</p>
									<div className="flex flex-wrap gap-1.5">
										{selectedLanguageInfo && (
											<Badge variant="secondary" className="gap-1.5">
												<img
													src={selectedLanguageInfo.img}
													alt={selectedLanguageInfo.label}
													className="w-3.5 h-3.5 rounded-sm object-cover"
												/>
												{selectedLanguageInfo.label} node
											</Badge>
										)}
										{selectedFrameworkInfos.map((framework) => (
											<Badge
												key={framework.value}
												variant="secondary"
												className="gap-1"
											>
												<span>{framework.icon}</span>
												{framework.label} widgets
											</Badge>
										))}
									</div>
									<p className="text-xs text-muted-foreground/70">
										{selectedLanguageInfo && widgetFrameworks.length > 0
											? "Monorepo layout: node/ + widgets/, orchestrated by a root mise.toml."
											: selectedLanguageInfo
												? "Single node project — the template's standard layout."
												: "Widgets-only package: widgets/ per framework, packed into widgets.flwb."}
									</p>
								</div>

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
											placeholder="my-custom-package"
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
										onClick={() => setStep("capabilities")}
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
									Downloading the selected templates and scaffolding your
									project. This may take a moment.
								</p>
							</motion.div>
						)}
					</AnimatePresence>
				</div>
			</div>
		</div>
	);
}
