export function capturePage(find?: string): {
  url: string;
  title: string;
  blocks: [kind: string, depth: number, text: string][];
  truncated: boolean;
  alternate: string;
} | { title: string; pdf: true };
