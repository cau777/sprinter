import { MarkdownTextPrimitive } from "@assistant-ui/react-markdown";
import type { MarkdownTextPrimitiveProps } from "@assistant-ui/react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";

export default function MarkdownTextMath({
  components,
  componentsByLanguage,
}: {
  components: NonNullable<MarkdownTextPrimitiveProps["components"]>;
  componentsByLanguage: NonNullable<MarkdownTextPrimitiveProps["componentsByLanguage"]>;
}) {
  return <MarkdownTextPrimitive remarkPlugins={[remarkGfm, remarkMath]} rehypePlugins={[rehypeKatex]} components={components} componentsByLanguage={componentsByLanguage} defer smooth={false} />;
}
