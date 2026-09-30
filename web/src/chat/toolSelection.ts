export function setToolSelection(selected: readonly string[], toolId: string, enabled: boolean): string[] {
  return enabled ? [toolId] : selected.filter((id) => id !== toolId);
}
