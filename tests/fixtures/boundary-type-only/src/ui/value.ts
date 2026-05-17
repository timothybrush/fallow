// Plain value import. Must fire as a violation regardless of allowTypeOnly.
import { runQuery } from '../db/runtime';
import { helper } from '../shared/utils';

export const formatValue = (q: { sql: string }): string => helper() + runQuery(q);
