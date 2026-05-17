import type { Query } from './types';

export const runQuery = (q: Query): string => q.sql;

export const initDb = (): void => {};
