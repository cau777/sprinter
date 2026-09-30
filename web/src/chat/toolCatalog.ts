export const WEB_SEARCH = "openrouter:web_search";
export const BASH = "openrouter:bash";

export function toolLabel(id: string) {
  switch (id) {
    case WEB_SEARCH: return "Web search";
    case BASH: return "Bash";
    case "openrouter:image_generation": return "Image generation";
    case "openrouter:advisor": return "Advisor";
    case "openrouter:subagent": return "Subagent";
    case "openrouter:apply_patch": return "Apply patch";
    case "openrouter:web_fetch": return "Web fetch";
    case "openrouter:shell": return "Shell";
    case "openrouter:fusion": return "Fusion";
    case "openrouter:experimental__search_models": return "Search models";
    case "openrouter:tool_search": return "Tool search";
    default: return id;
  }
}
