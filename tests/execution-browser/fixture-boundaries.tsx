export { cn } from "../../packages/ui/lib/utils";
export const i18n = { t: (_key: string, fallback: string) => fallback };
export const useTranslation = () => ({ t: i18n.t });
export function TextEditor({ initialContent }: { initialContent?: string }) {
	return <p className="leading-relaxed">{initialContent}</p>;
}
