export function capturePage(find?: string | null, about?: boolean): {
  url: string;
  title: string;
  blocks: [kind: string, depth: number, text: string][];
  truncated: boolean;
  alternate: string;
  signals?: Record<string, boolean | number>;
  meta?: { author: string | null; publication: string; publishDate: string | null };
} | { title: string; pdf: true };
