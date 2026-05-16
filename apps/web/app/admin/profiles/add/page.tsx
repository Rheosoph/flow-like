"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	type IBit,
	IBitTypes,
	IConnectionMode,
	type IProfile,
	useBackend,
} from "@tm9657/flow-like-ui";
import { Button } from "@tm9657/flow-like-ui";
import { Input } from "@tm9657/flow-like-ui";
import { Label } from "@tm9657/flow-like-ui";
import { Textarea } from "@tm9657/flow-like-ui";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@tm9657/flow-like-ui";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@tm9657/flow-like-ui";
import { Badge } from "@tm9657/flow-like-ui";
import { Avatar, AvatarFallback, AvatarImage } from "@tm9657/flow-like-ui";
import { Separator } from "@tm9657/flow-like-ui";
import { Image, Monitor, Plus, Save, Upload, User, X } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { useApi } from "../../../../lib/useApi";

const DEFAULT_PROFILE: IProfile = {
	apps: [],
	bits: [],
	created: new Date().toISOString(),
	description: null,
	hub: "",
	hubs: [],
	icon: null,
	id: createId(),
	interests: [],
	name: "",
	settings: {
		connection_mode: IConnectionMode.Simplebezier,
	},
	updated: new Date().toISOString(),
	tags: [],
	thumbnail: null,
};

export default function AddProfilePage() {
	const [profile, setProfile] = useState<IProfile>(DEFAULT_PROFILE);
	const [newTag, setNewTag] = useState("");
	const [newInterest, setNewInterest] = useState("");
	const [newHub, setNewHub] = useState("");
	const [newBitId, setNewBitId] = useState("");
	const iconInputRef = useRef<HTMLInputElement>(null);
	const thumbnailInputRef = useRef<HTMLInputElement>(null);
	const backend = useBackend();

	const bits = useApi<IBit[]>(
		"POST",
		"bit",
		{
			bit_types: [
				IBitTypes.Embedding,
				IBitTypes.ImageEmbedding,
				IBitTypes.Llm,
				IBitTypes.Stt,
				IBitTypes.Tts,
				IBitTypes.Vlm,
			],
		},
		true,
	);

	const updateProfile = (field: keyof IProfile, value: any) => {
		setProfile((prev) => ({
			...prev,
			[field]: value,
			updated: new Date().toISOString(),
		}));
	};

	const addTag = () => {
		if (newTag.trim() && !profile?.tags?.includes(newTag.trim())) {
			updateProfile("tags", [...(profile?.tags ?? []), newTag.trim()]);
			setNewTag("");
		}
	};

	const removeTag = (tag: string) => {
		updateProfile(
			"tags",
			profile.tags?.filter((t) => t !== tag),
		);
	};

	const addInterest = () => {
		if (
			newInterest.trim() &&
			!profile.interests?.includes(newInterest.trim())
		) {
			updateProfile("interests", [
				...(profile.interests ?? []),
				newInterest.trim(),
			]);
			setNewInterest("");
		}
	};

	const removeInterest = (interest: string) => {
		updateProfile(
			"interests",
			profile.interests?.filter((i) => i !== interest),
		);
	};

	const addHub = () => {
		if (newHub.trim() && !profile.hubs?.includes(newHub.trim())) {
			updateProfile("hubs", [...(profile.hubs ?? []), newHub.trim()]);
			setNewHub("");
		}
	};

	const removeHub = (hub: string) => {
		updateProfile(
			"hubs",
			profile.hubs?.filter((h) => h !== hub),
		);
	};

	const addBitId = () => {
		if (newBitId.trim() && !profile.bits?.includes(newBitId.trim())) {
			updateProfile("bits", [...(profile.bits ?? []), newBitId.trim()]);
			setNewBitId("");
		}
	};

	const removeBitId = (bitId: string) => {
		updateProfile(
			"bits",
			profile.bits?.filter((b) => b !== bitId),
		);
	};

	const toggleBitId = (bitId: string) => {
		const currentBits = profile.bits ?? [];
		if (currentBits.includes(bitId)) {
			updateProfile(
				"bits",
				currentBits.filter((b) => b !== bitId),
			);
		} else {
			updateProfile("bits", [...currentBits, bitId]);
		}
	};

	const handleSave = useCallback(async () => {
		await backend.apiState.put(
			profile,
			`admin/profiles/${profile.id}`,
			profile,
		);

		setProfile(DEFAULT_PROFILE);
	}, [profile, backend.apiState]);

	const handleImageUpload = useCallback(
		async (
			event: React.ChangeEvent<HTMLInputElement>,
			type: "icon" | "thumbnail",
		) => {
			const file = event.target.files?.[0];
			if (file && file.type === "image/webp") {
				const signedUrl = await backend.apiState.get<{
					url: string;
					final_url?: string;
				}>(profile, "admin/profiles/media");
				if (signedUrl?.url) {
					const response = await fetch(signedUrl.url, {
						method: "PUT",
						body: file,
						headers: {
							"Content-Type": file.type,
						},
					});

					if (response.ok) {
						const imageUrl = signedUrl.final_url
							? signedUrl.final_url
							: signedUrl.url.split("?")[0];
						updateProfile(type, imageUrl);
					} else {
					}
				} else {
				}
			} else if (file) {
				if (type === "icon" && iconInputRef.current) {
					iconInputRef.current.value = "";
				} else if (type === "thumbnail" && thumbnailInputRef.current) {
					thumbnailInputRef.current.value = "";
				}
			}
		},
		[backend.apiState, profile],
	);

	return (
		<div className="container mx-auto p-6 max-w-4xl">
			<div className="mb-6">
				<h1 className="text-3xl font-bold">Create New Profile</h1>
				<p className="text-muted-foreground">
					Configure a new user profile with custom settings and preferences.
				</p>
			</div>

			<div className="grid gap-6">
				{/* Basic Information */}
				<Card>
					<CardHeader>
						<CardTitle className="flex items-center gap-2">
							<User className="h-5 w-5" />
							Basic Information
						</CardTitle>
						<CardDescription>
							Set up the fundamental profile details and appearance.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						{/* Profile Images */}
						<div className="grid grid-cols-1 md:grid-cols-2 gap-6">
							{/* Icon Upload (1:1) */}
							<div className="space-y-3">
								<Label>Profile Icon (1:1)</Label>
								<div className="flex items-center gap-4">
									<Avatar className="h-20 w-20 rounded-md">
										<AvatarImage
											className="object-cover"
											src={profile.icon ?? "/placeholder.webp"}
										/>
										<AvatarFallback>
											<User className="h-8 w-8" />
										</AvatarFallback>
									</Avatar>
									<div className="flex-1">
										<input
											ref={iconInputRef}
											type="file"
											accept="image/webp"
											onChange={(e) => handleImageUpload(e, "icon")}
											className="hidden"
										/>
										<Button
											variant="outline"
											className="w-full"
											onClick={() => iconInputRef.current?.click()}
										>
											<Upload className="h-4 w-4 mr-2" />
											Upload Icon
										</Button>
									</div>
								</div>
							</div>

							{/* Thumbnail Upload (16:9) */}
							<div className="space-y-3">
								<Label>Profile Thumbnail (16:9)</Label>
								<div className="space-y-3">
									<div className="aspect-video w-full bg-muted rounded-lg overflow-hidden">
										{profile.thumbnail ? (
											<img
												src={profile.thumbnail}
												alt="Profile thumbnail"
												className="w-full h-full object-cover"
											/>
										) : (
											<div className="w-full h-full flex items-center justify-center">
												<Image className="h-8 w-8 text-muted-foreground" />
											</div>
										)}
									</div>
									<input
										ref={thumbnailInputRef}
										type="file"
										accept="image/webp"
										onChange={(e) => handleImageUpload(e, "thumbnail")}
										className="hidden"
									/>
									<Button
										variant="outline"
										className="w-full"
										onClick={() => thumbnailInputRef.current?.click()}
									>
										<Upload className="h-4 w-4 mr-2" />
										Upload Thumbnail
									</Button>
								</div>
							</div>
						</div>

						<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
							<div className="space-y-2">
								<Label htmlFor="name">Profile Name</Label>
								<Input
									id="name"
									value={profile.name}
									onChange={(e) => updateProfile("name", e.target.value)}
									placeholder="Enter profile name"
								/>
							</div>
							<div className="space-y-2">
								<Label htmlFor="hub">Primary Hub</Label>
								<Input
									id="hub"
									value={profile.hub}
									onChange={(e) => updateProfile("hub", e.target.value)}
									placeholder="Enter hub URL or identifier"
								/>
							</div>
						</div>

						<div className="space-y-2">
							<Label htmlFor="description">Description</Label>
							<Textarea
								id="description"
								value={profile.description || ""}
								onChange={(e) =>
									updateProfile("description", e.target.value || null)
								}
								placeholder="Describe this profile..."
								rows={3}
							/>
						</div>
					</CardContent>
				</Card>

				{/* Hubs Management */}
				<Card>
					<CardHeader>
						<CardTitle className="flex items-center gap-2">
							<Monitor className="h-5 w-5" />
							Hub Management
						</CardTitle>
						<CardDescription>
							Manage additional hubs associated with this profile.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="flex gap-2 flex-row items-center">
							<Input
								value={newHub}
								onChange={(e) => setNewHub(e.target.value)}
								placeholder="Add a hub URL..."
								onKeyPress={(e) => e.key === "Enter" && addHub()}
							/>
							<Button onClick={addHub} size="sm">
								<Plus className="h-4 w-4" />
							</Button>
						</div>

						{(profile.hubs?.length ?? 0) > 0 && (
							<div className="space-y-2">
								{profile.hubs?.map((hub) => (
									<div
										key={hub}
										className="flex items-center justify-between p-2 border rounded"
									>
										<span className="text-sm font-mono">{hub}</span>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => removeHub(hub)}
										>
											<X className="h-4 w-4" />
										</Button>
									</div>
								))}
							</div>
						)}
					</CardContent>
				</Card>

				{/* Bit IDs Management */}
				<Card>
					<CardHeader>
						<CardTitle>Bit IDs</CardTitle>
						<CardDescription>
							Select and manage bit IDs associated with this profile.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						{/* Dropdown Bit Selection */}
						{bits.data && bits.data.length > 0 && (
							<div className="space-y-3">
								<Label>Add Bit from Available</Label>
								<Select
									value=""
									onValueChange={(bitId) => {
										if (bitId && !profile.bits?.includes(bitId)) {
											updateProfile("bits", [...(profile.bits ?? []), bitId]);
										}
									}}
								>
									<SelectTrigger>
										<SelectValue placeholder="Select a bit to add..." />
									</SelectTrigger>
									<SelectContent>
										{bits.data
											?.filter(
												(bit) =>
													!profile.bits?.includes(`${bit.hub}:${bit.id}`),
											)
											.map((bit) => (
												<SelectItem key={bit.id} value={`${bit.hub}:${bit.id}`}>
													<div className="flex flex-col">
														<span className="font-medium">
															{bit.meta?.en?.name ?? bit.id}
														</span>
														<span className="text-xs text-muted-foreground font-mono">
															{bit.id}
														</span>
													</div>
												</SelectItem>
											))}
									</SelectContent>
								</Select>
							</div>
						)}

						{/* Manual Bit ID Entry */}
						<div className="flex gap-2 flex-row items-center">
							<Input
								value={newBitId}
								onChange={(e) => setNewBitId(e.target.value)}
								placeholder="Add a bit ID manually..."
								onKeyPress={(e) => e.key === "Enter" && addBitId()}
							/>
							<Button onClick={addBitId} size="sm">
								<Plus className="h-4 w-4" />
							</Button>
						</div>

						{/* Selected Bit IDs */}
						{(profile.bits?.length ?? 0) > 0 && (
							<div className="space-y-2">
								<Label>Selected Bit IDs</Label>
								<div className="flex flex-wrap gap-2">
									{profile.bits?.map((bitId) => {
										const bit = bits.data?.find(
											(b) => `${b.hub}:${b.id}` === bitId,
										);
										const displayName = bit?.meta?.en?.name || bitId;

										return (
											<Badge
												key={bitId}
												variant="outline"
												className="flex items-center gap-1"
											>
												<div className="flex flex-col">
													<span className="text-xs">{displayName}</span>
													{bit?.meta?.en?.name && (
														<span className="text-xs font-mono text-muted-foreground">
															{bitId}
														</span>
													)}
												</div>
												<button
													onClick={() => removeBitId(bitId)}
													className="ml-1"
												>
													<X className="h-3 w-3" />
												</button>
											</Badge>
										);
									})}
								</div>
							</div>
						)}
					</CardContent>
				</Card>

				{/* Settings */}
				<Card>
					<CardHeader>
						<CardTitle>Connection Settings</CardTitle>
						<CardDescription>
							Configure how connections are displayed and managed.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<div className="space-y-2">
							<Label htmlFor="connection-mode">Connection Mode</Label>
							<Select
								value={profile.settings?.connection_mode}
								onValueChange={(value) =>
									updateProfile("settings", {
										...profile.settings,
										connection_mode: value as IConnectionMode,
									})
								}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{Object.values(IConnectionMode).map((mode) => (
										<SelectItem key={mode} value={mode}>
											{mode.replace(/([A-Z])/g, " $1").trim()}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					</CardContent>
				</Card>

				{/* Tags */}
				<Card>
					<CardHeader>
						<CardTitle>Tags</CardTitle>
						<CardDescription>
							Add tags to categorize and organize this profile.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="flex gap-2 flex-row items-center">
							<Input
								value={newTag}
								onChange={(e) => setNewTag(e.target.value)}
								placeholder="Add a tag..."
								onKeyPress={(e) => e.key === "Enter" && addTag()}
							/>
							<Button onClick={addTag} size="sm">
								<Plus className="h-4 w-4" />
							</Button>
						</div>

						{(profile.tags?.length ?? 0) > 0 && (
							<div className="flex flex-wrap gap-2 flex-row items-center">
								{profile.tags?.map((tag) => (
									<Badge
										key={tag}
										variant="secondary"
										className="flex items-center gap-1"
									>
										{tag}
										<button onClick={() => removeTag(tag)} className="ml-1">
											<X className="h-3 w-3" />
										</button>
									</Badge>
								))}
							</div>
						)}
					</CardContent>
				</Card>

				{/* Interests */}
				<Card>
					<CardHeader>
						<CardTitle>Interests</CardTitle>
						<CardDescription>
							Define interests and topics relevant to this profile.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="flex gap-2 flex-row items-center">
							<Input
								value={newInterest}
								onChange={(e) => setNewInterest(e.target.value)}
								placeholder="Add an interest..."
								onKeyPress={(e) => e.key === "Enter" && addInterest()}
							/>
							<Button onClick={addInterest} size="sm">
								<Plus className="h-4 w-4" />
							</Button>
						</div>

						{(profile.interests?.length ?? 0) > 0 && (
							<div className="flex flex-wrap gap-2">
								{profile.interests?.map((interest) => (
									<Badge
										key={interest}
										variant="outline"
										className="flex items-center gap-1"
									>
										{interest}
										<button
											onClick={() => removeInterest(interest)}
											className="ml-1"
										>
											<X className="h-3 w-3" />
										</button>
									</Badge>
								))}
							</div>
						)}
					</CardContent>
				</Card>

				{/* Profile Info */}
				<Card>
					<CardHeader>
						<CardTitle>Profile Information</CardTitle>
					</CardHeader>
					<CardContent>
						<div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
							<div>
								<Label className="text-muted-foreground">Profile ID</Label>
								<p className="font-mono">{profile.id}</p>
							</div>
							<div>
								<Label className="text-muted-foreground">Created</Label>
								<p>{new Date(profile.created).toLocaleString()}</p>
							</div>
						</div>
					</CardContent>
				</Card>
			</div>

			<Separator className="my-6" />

			{/* Actions */}
			<div className="flex justify-end gap-2">
				<Button variant="outline">Cancel</Button>
				<Button onClick={handleSave} className="flex items-center gap-2">
					<Save className="h-4 w-4" />
					Create Profile
				</Button>
			</div>
		</div>
	);
}
