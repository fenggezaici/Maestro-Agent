import { Children, useState, type ReactElement, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
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

export default function MarkdownMessage({ content, light }: { content: string; light: boolean }) {
  return <div className="markdown-message"><ReactMarkdown skipHtml components={{
    img: ({ alt }) => <span className="blocked-remote-media">[远程图片未加载：{alt || 'image'}]</span>,
    a: ({ children, href }) => <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
    pre: ({ children }) => {
      const child = Children.only(children) as ReactElement<{ className?: string; children?: ReactNode }>;
      const language = /language-([\w+-]+)/.exec(child.props.className ?? '')?.[1] ?? '';
      return <CodeBlock code={String(child.props.children ?? '').replace(/\n$/, '')} language={language} light={light} />;
    },
  }}>{content}</ReactMarkdown></div>;
}
