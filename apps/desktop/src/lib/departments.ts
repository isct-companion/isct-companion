/** Settings may hold a plain id ("cs") or a subscription-scoped one
 * ("2026:cs"). */
export const departmentById = <T extends { id: string }>(
  departments: readonly T[],
  id: string | undefined,
) =>
  id
    ? departments.find((d) => d.id === id || id.endsWith(`:${d.id}`))
    : undefined;
