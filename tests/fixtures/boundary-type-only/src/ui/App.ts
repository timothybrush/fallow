// Entry: keeps every importer file reachable so the boundary detector
// inspects each one. The `Query` type-only re-export is kept reachable by
// importing it back as a type from the type_reexport barrel.
import type { Query } from './type_reexport';

import { formatQuery } from './type_only';
import { formatId } from './inline_type';
import { sumIds } from './namespace_type';
import { formatMixed } from './mixed';
import { formatValue } from './value';
import { sideEffected } from './side_effect';
import { formatSibling } from './sibling_imports';

export const app = (): string => {
  const q: Query = { sql: 'reexport' };
  return [
    formatQuery({ sql: 'a' }),
    formatId({ sql: 'b' }),
    sumIds([1, 2]),
    formatMixed({ sql: 'c' }),
    formatValue({ sql: 'd' }),
    sideEffected(),
    formatSibling({ sql: 'e' }),
    q.sql,
  ].join(',');
};
