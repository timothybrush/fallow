// Whole-declaration `import type` to db. Allowed when db is in `allowTypeOnly`.
import type { Query } from '../db/types';
import { helper } from '../shared/utils';

export const formatQuery = (q: Query): string => helper() + q.sql;
