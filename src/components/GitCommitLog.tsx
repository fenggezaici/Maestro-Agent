import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { gitApi, type GitCommit, type GitFileEntry } from '../api/git';
import { relativeTime } from '../lib/conversation';
import { ChangeRow } from './GitPane';

/// commit 历史浏览：平面列表（非 DAG lane——merge 拓扑图在 agent 场景查阅率
/// 极低，完整图形渲染归 Roadmap）。行展开看详情 + 逐文件 diff 懒加载，
/// 分页"加载更多"。

const PAGE_SIZE = 100;

function shortHash(hash: string): string {
  return hash.slice(0, 7);
}

function RefBadges({ refs }: { refs: string[] }) {
  if (refs.length === 0) return null;
  return <span className="git-commit-refs">
    {refs.map((ref) => {
      const label = ref.replace(/^HEAD -> /, '');
      const head = ref.startsWith('HEAD');
      const tag = ref.startsWith('tag: ');
      return <em key={ref} className={`git-ref-badge${head ? ' head' : ''}${tag ? ' tag' : ''}`}>{tag ? label.replace(/^tag: /, '') : label}</em>;
    })}
  </span>;
}

function CommitDetail({ commit, fontSize }: { commit: GitCommit; fontSize: number }) {
  const [files, setFiles] = useState<GitFileEntry[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [expandedFile, setExpandedFile] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void gitApi.filesOf(`commit:${commit.hash}`)
      .then((result) => { if (!cancelled) setFiles(result ?? []); })
      .catch(() => { if (!cancelled) setFiles([]); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [commit.hash]);

  const toggleFile = useCallback((key: string) => {
    setExpandedFile((current) => (current === key ? null : key));
  }, []);

  return <div className="git-commit-detail">
    <div className="git-commit-meta">
      <span>{commit.author} &lt;{commit.email}&gt;</span>
      <span>{new Date(commit.timestamp * 1000).toLocaleString()}</span>
      {commit.parents.length > 0 && <span title={commit.parents.join(', ')}>父提交 {commit.parents.map(shortHash).join(', ')}</span>}
    </div>
    {loading && <p className="workspace-status" role="status">正在读取文件清单…</p>}
    {!loading && files && files.length === 0 && <p className="git-diff-empty">无文件清单（merge commit 的默认 diff 为空，v1 已知限制）</p>}
    {!loading && files?.map((entry) => <ChangeRow key={entry.path} entry={entry} column="staged"
      diffSource={`commit:${commit.hash}`}
      expanded={expandedFile === entry.path}
      onToggle={() => toggleFile(entry.path)} fontSize={fontSize} />)}
  </div>;
}

export default function GitCommitLog({ fontSize, active }: { fontSize: number; active: boolean }) {
  const [commits, setCommits] = useState<GitCommit[]>([]);
  const [loading, setLoading] = useState(false);
  const [exhausted, setExhausted] = useState(false);
  const [expandedHash, setExpandedHash] = useState<string | null>(null);
  const loadedRef = useRef(false);

  const loadPage = useCallback((skip: number) => {
    setLoading(true);
    void gitApi.log(PAGE_SIZE, skip)
      .then((page) => {
        const rows = page ?? [];
        setCommits((current) => (skip === 0 ? rows : [...current, ...rows]));
        if (rows.length < PAGE_SIZE) setExhausted(true);
      })
      .catch(() => { if (skip === 0) setCommits([]); })
      .finally(() => setLoading(false));
  }, []);

  // 仅在 tab 首次激活时拉取——切换来源不丢已加载的分页状态。
  useEffect(() => {
    if (!active || loadedRef.current) return;
    loadedRef.current = true;
    loadPage(0);
  }, [active, loadPage]);

  if (!loadedRef.current && !loading) return null;
  return <div className="git-commit-log">
    {commits.length === 0 && !loading && <p className="review-files-empty">没有提交历史</p>}
    {commits.map((commit) => {
      const expanded = expandedHash === commit.hash;
      return <div key={commit.hash} className={`git-commit-card${expanded ? ' expanded' : ''}`}>
        <button type="button" className="git-commit-row" aria-expanded={expanded} aria-label={`提交 ${shortHash(commit.hash)} ${commit.subject}`}
          onClick={() => setExpandedHash(expanded ? null : commit.hash)}>
          <span className="git-change-chevron">{expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}</span>
          <code className="git-commit-hash">{shortHash(commit.hash)}</code>
          <span className="git-commit-subject">{commit.subject}</span>
          <RefBadges refs={commit.refs} />
          <span className="git-commit-meta-inline">{commit.author} · {relativeTime(commit.timestamp)}</span>
        </button>
        {expanded && <CommitDetail commit={commit} fontSize={fontSize} />}
      </div>;
    })}
    {loading && <p className="workspace-status" role="status">正在读取提交历史…</p>}
    {!exhausted && commits.length > 0 && !loading &&
      <button type="button" className="button-ghost git-log-more" onClick={() => loadPage(commits.length)}>加载更多</button>}
  </div>;
}
