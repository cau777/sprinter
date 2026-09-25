export type TreeMessage = {
  id: string;
  parent_id: string | null;
  role: string;
  created_at: number;
};

export function visiblePath<T extends TreeMessage>(messages: T[], leafId: string | null): T[] {
  if (!leafId) return [];
  const byId = new Map(messages.map((message) => [message.id, message]));
  const path: T[] = [];
  const seen = new Set<string>();
  let cursor = byId.get(leafId);
  while (cursor && !seen.has(cursor.id)) {
    seen.add(cursor.id);
    path.unshift(cursor);
    cursor = cursor.parent_id ? byId.get(cursor.parent_id) : undefined;
  }
  return path;
}

export function siblingsFor<T extends TreeMessage>(messages: T[], message: T): T[] {
  return messages
    .filter((candidate) => candidate.parent_id === message.parent_id && candidate.role === message.role)
    .sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));
}
