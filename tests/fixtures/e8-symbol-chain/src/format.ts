// The traced symbol. Imported by report.ts and middle.ts.
export function formatDate(value: number): string {
  return new Date(value).toISOString();
}
