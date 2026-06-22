import { formatDate } from "./format";

// A SECOND caller of formatDate, so the hand-computed caller set has two
// importers (report.ts and middle.ts).
export function describe(ts: number): string {
  return `at ${formatDate(ts)}`;
}
