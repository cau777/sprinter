import { Suspense, lazy, useDeferredValue, useEffect, useId, useState, type ComponentPropsWithoutRef } from "react";
import { Button } from "@heroui/react";
import { useMessagePartText, useAuiState } from "@assistant-ui/react";
import type { CodeHeaderProps, SyntaxHighlighterProps } from "@assistant-ui/react-markdown";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

const MathMarkdown = lazy(() => import("./MarkdownTextMath"));

const theme = {
  name: "sprinter-night",
  type: "dark",
  colors: { "editor.background": "#101827", "editor.foreground": "#dbeafe" },
  tokenColors: [
    { scope: ["keyword", "storage"], settings: { foreground: "#79d9ff" } },
    { scope: ["string", "constant.numeric"], settings: { foreground: "#a5e3bd" } },
    { scope: ["comment"], settings: { foreground: "#8493a8", fontStyle: "italic" } },
    { scope: ["entity.name.function", "support.function"], settings: { foreground: "#9bbcff" } },
  ],
};

const grammarLoaders: Record<string, () => Promise<{ default: never }>> = {
  bash: () => import("shiki/langs/bash.mjs") as Promise<{ default: never }>,
  css: () => import("shiki/langs/css.mjs") as Promise<{ default: never }>,
  html: () => import("shiki/langs/html.mjs") as Promise<{ default: never }>,
  javascript: () => import("shiki/langs/javascript.mjs") as Promise<{ default: never }>,
  js: () => import("shiki/langs/javascript.mjs") as Promise<{ default: never }>,
  json: () => import("shiki/langs/json.mjs") as Promise<{ default: never }>,
  python: () => import("shiki/langs/python.mjs") as Promise<{ default: never }>,
  py: () => import("shiki/langs/python.mjs") as Promise<{ default: never }>,
  rust: () => import("shiki/langs/rust.mjs") as Promise<{ default: never }>,
  ts: () => import("shiki/langs/typescript.mjs") as Promise<{ default: never }>,
  typescript: () => import("shiki/langs/typescript.mjs") as Promise<{ default: never }>,
  yaml: () => import("shiki/langs/yaml.mjs") as Promise<{ default: never }>,
};

const highlighters = new Map<string, Promise<import("shiki/core").HighlighterCore>>();
function highlighterFor(language: string) {
  const lang = language.toLowerCase();
  const loader = grammarLoaders[lang];
  if (!loader) return Promise.resolve(undefined);
  let pending = highlighters.get(lang);
  if (!pending) {
    pending = Promise.all([import("shiki/core"), import("shiki/engine/oniguruma"), loader()]).then(async ([shiki, engine, grammar]) => {
      return shiki.createHighlighterCore({ engine: await engine.createOnigurumaEngine(), themes: [theme as import("shiki").ThemeRegistration], langs: [grammar.default as never] });
    });
    highlighters.set(lang, pending);
  }
  return pending;
}

function CopyCode({ language, code }: CodeHeaderProps) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    setCopied(true);
    const fallback = () => {
      const field = document.createElement("textarea");
      field.value = code;
      field.style.position = "fixed";
      field.style.opacity = "0";
      document.body.appendChild(field);
      field.select();
      document.execCommand("copy");
      field.remove();
    };
    try {
      if (navigator.clipboard?.writeText) void navigator.clipboard.writeText(code).catch(fallback);
      else fallback();
    } catch {
      fallback();
    }
    window.setTimeout(() => setCopied(false), 1800);
  };
  return <div className="markdown-code-header"><span>{language ?? "CODE"}</span><Button data-testid="copy-code-button" variant="ghost" className="rounded px-2 py-1 font-mono text-[9px] text-slate-300 hover:bg-white/10" onPress={copy}>{copied ? "Copied" : "Copy"}</Button></div>;
}

function SafeLink({ children, href, ...props }: ComponentPropsWithoutRef<"a">) {
  return <a {...props} href={href} target="_blank" rel="noopener noreferrer">{children}</a>;
}

function HighlightedCode({ components: { Pre }, language, code }: SyntaxHighlighterProps) {
  const [html, setHtml] = useState<string>();
  useEffect(() => {
    let active = true;
    setHtml(undefined);
    void highlighterFor(language).then((highlighter) => {
      if (active && highlighter) setHtml(highlighter.codeToHtml(code, { lang: language.toLowerCase(), theme: "sprinter-night" }));
    }).catch(() => undefined);
    return () => { active = false; };
  }, [code, language]);
  if (!html) return <Pre><code>{code}</code></Pre>;
  return <div className="markdown-highlight" dangerouslySetInnerHTML={{ __html: html }} />;
}

function MermaidDiagram({ components: { Pre }, code }: SyntaxHighlighterProps) {
  const status = useAuiState((state) => state.message.status?.type);
  const complete = status !== "running";
  const id = `sprinter-mermaid-${useId().replace(/:/g, "")}`;
  const [svg, setSvg] = useState<string>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!complete) return;
    let active = true;
    setSvg(undefined);
    setFailed(false);
    void import("mermaid").then(async ({ default: mermaid }) => {
      mermaid.initialize({ securityLevel: "strict", theme: "dark", startOnLoad: false });
      const result = await mermaid.render(id, code);
      if (active) setSvg(result.svg);
    }).catch(() => { if (active) setFailed(true); });
    return () => { active = false; };
  }, [code, complete, id]);
  if (!complete || failed || !svg) return <Pre><code className="language-mermaid">{code}</code></Pre>;
  return <div className="mermaid-diagram" role="img" aria-label="Mermaid diagram" dangerouslySetInnerHTML={{ __html: svg }} />;
}

const codeComponents = {
  Pre: ({ children }: ComponentPropsWithoutRef<"pre">) => <pre>{children}</pre>,
  Code: ({ children, className }: ComponentPropsWithoutRef<"code">) => <code className={className}>{children}</code>,
};

function MarkdownCode({ className, children }: ComponentPropsWithoutRef<"code">) {
  const raw = String(children);
  const code = raw.replace(/\n$/, "");
  const language = /language-([^\s]+)/.exec(className ?? "")?.[1];
  const block = Boolean(language) || raw.endsWith("\n");
  if (!block) return <code className={className}>{children}</code>;
  if (language === "mermaid") {
    return <MermaidDiagram components={codeComponents} language="mermaid" code={code} />;
  }
  return <div className="markdown-code-block">
    <CopyCode language={language} code={code} />
    <HighlightedCode components={codeComponents} language={language ?? "text"} code={code} />
  </div>;
}

const baseComponents = {
  a: SafeLink,
  code: MarkdownCode,
  pre: ({ children }: ComponentPropsWithoutRef<"pre">) => <>{children}</>,
} satisfies NonNullable<Components>;

export function MarkdownContent({ text }: { text: string }) {
  const deferredText = useDeferredValue(text);
  const hasMath = /(?:\$\$?[\s\S]+?\$\$?|\\\([\s\S]+?\\\)|\\\[[\s\S]+?\\\])/.test(deferredText);
  const fallback = <ReactMarkdown remarkPlugins={[remarkGfm]} components={baseComponents}>{deferredText}</ReactMarkdown>;
  return hasMath
    ? <Suspense fallback={fallback}><MathMarkdown content={deferredText} components={baseComponents} /></Suspense>
    : fallback;
}

export function MarkdownText() {
  const text = useMessagePartText();
  return <MarkdownContent text={text.text} />;
}
