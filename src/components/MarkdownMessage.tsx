import { Children, memo, useMemo, useState, type ReactElement, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Light as SyntaxHighlighter } from 'react-syntax-highlighter';
import { atomOneDark, atomOneLight } from 'react-syntax-highlighter/dist/esm/styles/hljs';
import javascript from 'react-syntax-highlighter/dist/esm/languages/hljs/javascript';
import typescript from 'react-syntax-highlighter/dist/esm/languages/hljs/typescript';
import python from 'react-syntax-highlighter/dist/esm/languages/hljs/python';
import cpp from 'react-syntax-highlighter/dist/esm/languages/hljs/cpp';
import bash from 'react-syntax-highlighter/dist/esm/languages/hljs/bash';
import json from 'react-syntax-highlighter/dist/esm/languages/hljs/json';
import { Check, Copy } from 'lucide-react';

for (const [name, grammar] of Object.entries({ javascript, typescript, python, cpp, bash, json })) SyntaxHighlighter.registerLanguage(name, grammar);

/// Stable plugin list: a fresh array would re-parse every message on each render.
const REMARK_PLUGINS = [remarkGfm];

function CodeBlock({ code, language, light }: { code: string; language: string; light: boolean }) {
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);
  const aliases: Record<string, string> = { js: 'javascript', ts: 'typescript', sh: 'bash', 'c++': 'cpp' };
  const name = aliases[language] ?? language;
  return <div className="message-code-block"><header><span>{language || 'text'}</span><button type="button" aria-label="复制代码" onClick={async () => {
    try { await navigator.clipboard.writeText(code); setCopied(true); setFailed(false); } catch { setFailed(true); }
  }}>{copied ? <Check size={12} /> : <Copy size={12} />}{copied ? 'Copied' : 'Copy'}</button></header>
    <SyntaxHighlighter language={['javascript', 'typescript', 'python', 'cpp', 'bash', 'json'].includes(name) ? name : 'text'} style={light ? atomOneLight : atomOneDark} customStyle={{ margin: 0, background: 'transparent', fontFamily: 'var(--code-font)', fontSize: 'var(--code-size)' }} wrapLongLines={false}>{code}</SyntaxHighlighter>
    {failed && <span className="clipboard-error">无法访问剪贴板，请手动复制。</span>}
  </div>;
}

function MarkdownMessage({ content, light }: { content: string; light: boolean }) {
  // Component types must keep their identity: a fresh object here makes React
  // unmount and rebuild every code block (and re-run highlighting) on render.
  const components = useMemo(() => ({
    img: ({ alt }: { alt?: string }) => <span className="blocked-remote-media">[远程图片未加载：{alt || 'image'}]</span>,
    a: ({ children, href }: { children?: ReactNode; href?: string }) => <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
    pre: ({ children }: { children?: ReactNode }) => {
      const child = Children.only(children) as ReactElement<{ className?: string; children?: ReactNode }>;
      const language = /language-([\w+-]+)/.exec(child.props.className ?? '')?.[1] ?? '';
      return <CodeBlock code={String(child.props.children ?? '').replace(/\n$/, '')} language={language} light={light} />;
    },
  }), [light]);
  return <div className="markdown-message"><ReactMarkdown skipHtml remarkPlugins={REMARK_PLUGINS} components={components}>{content}</ReactMarkdown></div>;
}

/// Streaming re-renders the timeline on every token; only the growing message
/// should re-parse, so unchanged history skips markdown + highlighting.
export default memo(MarkdownMessage);
