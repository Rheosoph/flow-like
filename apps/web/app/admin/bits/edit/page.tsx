"use client";

import {
	Badge,
	BentoGrid,
	BitCard,
	BitTypeIcon,
	Button,
	Card,
	CardContent,
	type IBit,
	IBitTypes,
	Input,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	bitTypeToText,
} from "@flow-like/flow-like-ui";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ChevronLeft,
	ChevronRight,
	ChevronsLeft,
	Filter,
	Search,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useApi } from "../../../../lib/useApi";

const ITEMS_PER_PAGE_OPTIONS = [12, 24, 48, 96];

const ALL_BIT_TYPES = [
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.Tts,
	IBitTypes.Stt,
	IBitTypes.Embedding,
	IBitTypes.ImageEmbedding,
	IBitTypes.File,
	IBitTypes.Media,
	IBitTypes.Template,
	IBitTypes.Tokenizer,
	IBitTypes.TokenizerConfig,
	IBitTypes.SpecialTokensMap,
	IBitTypes.Config,
	IBitTypes.Course,
	IBitTypes.PreprocessorConfig,
	IBitTypes.Projection,
	IBitTypes.Project,
	IBitTypes.Board,
	IBitTypes.Other,
	IBitTypes.ObjectDetection,
];

let counter = 0;

export default function EditPage() {
	const [searchTerm, setSearchTerm] = useState("");
	const debouncedSearch = useDebounce(searchTerm, 300);
	const [selectedBitTypes, setSelectedBitTypes] = useState<IBitTypes[]>([
		IBitTypes.Llm,
		IBitTypes.Vlm,
		IBitTypes.Tts,
		IBitTypes.Stt,
		IBitTypes.Embedding,
		IBitTypes.ImageEmbedding,
	]);
	const [currentPage, setCurrentPage] = useState(1);
	const [itemsPerPage, setItemsPerPage] = useState(24);

	const queryParams = useMemo(
		() => ({
			search:
				debouncedSearch.trim() === ""
					? undefined
					: debouncedSearch || undefined,
			limit: itemsPerPage,
			offset: (currentPage - 1) * itemsPerPage,
			bit_types: selectedBitTypes.length > 0 ? selectedBitTypes : undefined,
		}),
		[debouncedSearch, selectedBitTypes, currentPage, itemsPerPage],
	);

	const bits = useApi<IBit[]>("POST", "bit", queryParams, true);

	useEffect(() => {
		console.dir(bits.data, "Bits data fetched in EditPage");
	}, [bits.data]);

	useEffect(() => {
		setCurrentPage(1);
	}, [searchTerm, selectedBitTypes, itemsPerPage]);

	const handleBitTypeToggle = (bitType: IBitTypes) => {
		setSelectedBitTypes((prev) =>
			prev.includes(bitType)
				? prev.filter((type) => type !== bitType)
				: [...prev, bitType],
		);
	};

	const handleSelectAllBitTypes = () => {
		setSelectedBitTypes(ALL_BIT_TYPES);
	};

	const handleClearBitTypes = () => {
		setSelectedBitTypes([]);
	};

	const totalItems = useMemo(() => {
		return bits.data?.length || 0;
	}, [bits.data]);

	const hasMorePages = useMemo(() => {
		// If we got exactly the number of items we requested, there might be more
		return totalItems === itemsPerPage;
	}, [totalItems, itemsPerPage]);

	const paginatedBits = useMemo(() => {
		if (!bits.data) return [];
		return bits.data.filter((bit) => bit.meta["en"]);
	}, [bits.data]);

	const handleNextPage = () => {
		if (hasMorePages) {
			setCurrentPage((prev) => prev + 1);
		}
	};

	const handlePrevPage = () => {
		setCurrentPage((prev) => Math.max(1, prev - 1));
	};

	const handleFirstPage = () => {
		setCurrentPage(1);
	};

	return (
		<main className="flex grow h-full min-h-0 bg-background overflow-hidden flex-col w-full">
			<div className="flex-1 min-h-0 overflow-y-auto p-6 space-y-6">
				{/* Search and Filters */}
				<Card className="w-full">
					<CardContent className="p-6 space-y-4">
						{/* Search Bar and Items Per Page */}
						<div className="flex items-center gap-4">
							<div className="relative flex-1">
								<Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-muted-foreground h-4 w-4" />
								<Input
									placeholder="Search bits..."
									value={searchTerm}
									onChange={(e) => setSearchTerm(e.target.value)}
									className="pl-10"
								/>
							</div>
							<div className="flex items-center gap-2">
								<span className="text-sm font-medium whitespace-nowrap">
									Items per page:
								</span>
								<Select
									value={itemsPerPage.toString()}
									onValueChange={(value) => setItemsPerPage(Number(value))}
								>
									<SelectTrigger className="w-20">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{ITEMS_PER_PAGE_OPTIONS.map((option) => (
											<SelectItem key={option} value={option.toString()}>
												{option}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
						</div>

						{/* Bit Type Filters */}
						<div className="space-y-3">
							<div className="flex items-end justify-between">
								<div className="flex items-center gap-2">
									<Filter className="h-4 w-4" />
									<span className="text-sm font-medium">Filter by Type:</span>
								</div>
								<div className="flex gap-2">
									<Button
										variant="outline"
										size="default"
										onClick={handleSelectAllBitTypes}
									>
										Select All
									</Button>
									<Button
										variant="outline"
										size="default"
										onClick={handleClearBitTypes}
									>
										Clear All
									</Button>
								</div>
							</div>
							<div className="w-full flex flex-wrap gap-2 items-center flex-row flex-between">
								{ALL_BIT_TYPES.map((bitType) => (
									<Badge
										key={bitType}
										variant={
											selectedBitTypes.includes(bitType)
												? "default"
												: "secondary"
										}
										className={`group cursor-pointer hover:bg-primary gap-2 ${selectedBitTypes.includes(bitType) ? "text-primary-foreground" : "text-secondary-foreground"}`}
										onClick={() => handleBitTypeToggle(bitType)}
									>
										<BitTypeIcon
											type={bitType}
											className={`w-3 h-3 group-hover:text-primary-foreground ${selectedBitTypes.includes(bitType) ? "text-primary-foreground" : "text-secondary-foreground"}`}
										/>
										<p
											className={`group-hover:text-primary-foreground ${selectedBitTypes.includes(bitType) ? "text-primary-foreground" : "text-secondary-foreground"}`}
										>
											{bitTypeToText(bitType)}
										</p>
									</Badge>
								))}
							</div>
						</div>
					</CardContent>
				</Card>

				{/* Results Summary */}
				<div className="text-sm text-muted-foreground">
					{bits.isLoading
						? "Loading..."
						: `Showing ${paginatedBits.length} items`}
				</div>

				{/* Bits Grid */}
				<div className="flex-1 w-full overflow-auto">
					{bits.data && paginatedBits.length > 0 ? (
						<BentoGrid className="mx-auto cursor-pointer w-full pb-20">
							{paginatedBits.map((bit, i) => {
								if (i === 0) counter = 0;
								const wide = counter === 3 || counter === 6;
								if (counter === 6) counter = 0;
								else counter += 1;
								return <BitCard key={bit.id} bit={bit} wide={wide} />;
							})}
						</BentoGrid>
					) : bits.isLoading ? (
						<div className="flex items-center justify-center h-32">
							<div className="text-muted-foreground">Loading bits...</div>
						</div>
					) : (
						<div className="flex items-center justify-center h-32">
							<div className="text-muted-foreground">
								No bits found matching your criteria
							</div>
						</div>
					)}
				</div>

				{/* Pagination */}
				{(currentPage > 1 || hasMorePages) && (
					<Card className="w-full">
						<CardContent className="p-4">
							<div className="flex items-center justify-between">
								<div className="text-sm text-muted-foreground">
									Page {currentPage} {hasMorePages ? "(more available)" : ""}
								</div>
								<div className="flex items-center gap-2">
									<Button
										variant="outline"
										size="sm"
										onClick={handleFirstPage}
										disabled={currentPage === 1}
									>
										<ChevronsLeft className="h-4 w-4" />
									</Button>
									<Button
										variant="outline"
										size="sm"
										onClick={handlePrevPage}
										disabled={currentPage === 1}
									>
										<ChevronLeft className="h-4 w-4" />
									</Button>
									<span className="text-sm px-2">Page {currentPage}</span>
									<Button
										variant="outline"
										size="sm"
										onClick={handleNextPage}
										disabled={!hasMorePages}
									>
										<ChevronRight className="h-4 w-4" />
									</Button>
								</div>
							</div>
						</CardContent>
					</Card>
				)}
			</div>
		</main>
	);
}
