export function capturePage(): {
  url: string;
  title: string;
  blocks: [kind: string, depth: number, text: string][];
  truncated: boolean;
  alternate: string;
};
