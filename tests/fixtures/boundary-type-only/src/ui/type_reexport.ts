// Type-only re-export. Surfaces as a boundary edge whose only symbol
// carries is_type_only=true, so the edge classifies as all-type-only and
// allowTypeOnly admits it just like an `import type` would.
export type { Query } from '../db/types';
