// Per-specifier inline `type` qualifier; the only specifier is type-only.
// Whole edge classifies as all-type-only.
import { type Query } from '../db/types';
import { helper } from '../shared/utils';

export const formatId = (q: Query): string => helper() + q.sql;
