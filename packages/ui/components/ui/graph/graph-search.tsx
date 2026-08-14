"use client";

import { useTranslation } from "@flow-like/locales";
import { Search } from "lucide-react";
import { useState } from "react";
import { Button } from "../button";
import { Input } from "../input";

export interface GraphSearchProps {
	onSearch: (query: string) => void;
	placeholder?: string;
}

export function GraphSearch({ onSearch, placeholder }: GraphSearchProps) {
	const { t } = useTranslation("common");
	const [query, setQuery] = useState("");

	const handleSubmit = (e: React.FormEvent) => {
		e.preventDefault();
		if (query.trim()) {
			onSearch(query.trim());
		}
	};

	return (
		<form onSubmit={handleSubmit} className="flex gap-2">
			<div className="relative flex-1">
				<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground pointer-events-none" />
				<Input
					value={query}
					onChange={(e) => setQuery(e.target.value)}
					placeholder={placeholder ?? t('searchNodes', 'Search nodes...')}
					className="pl-8 h-9"
				/>
			</div>
			<Button type="submit" size="sm" variant="secondary">
				{t('find', 'Find')}
			</Button>
		</form>
	);
}
