import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";

export default function MarkdownTextMath({
  content,
  components,
}: {
  content: string;
  components: NonNullable<Components>;
}) {
  return <ReactMarkdown remarkPlugins={[remarkGfm, remarkMath]} rehypePlugins={[rehypeKatex]} components={components}>{content}</ReactMarkdown>;
}
