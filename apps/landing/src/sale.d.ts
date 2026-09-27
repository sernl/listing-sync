// Types for the landing's sale script, which `web/src/lib/sale.test.ts` holds
// to the console's own sums. The landing itself is plain JavaScript.
export interface LandingSale {
	percent_off: number;
	until: string | null;
	banner: string;
	banner_href: string | null;
}
export function afterPercentOff(cents: number, percent: number): number;
export function saleLine(sale: LandingSale): string;
export function applySale(root: ParentNode, sale: LandingSale | null, dollars: (cents: number) => string): void;
export function loadSale(root: ParentNode, dollars: (cents: number) => string): Promise<void>;
