import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, FileCode2, GitBranch, RefreshCw } from 'lucide-react';
import { gitApi, type GitFileEntry, type GitStatus, type GitSummary } from '../api/git';
import type { GitRepoState } from '../hooks/useGitRepo';
import GitDiffView from './GitDiffView';
import GitCommitLog from './GitCommitLog';

/// ZCode GitPane 的只读子集：来源切换（未暂存/已暂存/与上游对比）+ 分区列表 +
/// diff 懒加载展开。明确偏离 ZCode 的两处：不引入虚拟滚动（<200 直接渲染，
/// 超出截断提示——agent 场景的文件列表极少突破此量级）；untracked 文件无 diff
/// （git 对工作区外文件不产生 diff，--no-index 方案在 Windows 有 /dev/null 兼容坑）。

const ROW_CAP = 200;

type SourceKey = 'unstaged' | 'staged' | 'branch' | 'log';

interface DiffLoadState {
  loading: boolean;
  text: string | null;
  truncated: boolean;
  binary: boolean;
}

function letterOf(entry: GitFileEntry, column: 'staged' | 'unstaged'): string {
  if (entry.xy === '?') return '?';
  const letter = column === 'staged' ? entry.xy[0] : entry.xy[1];
  return letter && letter !== '.' ? letter : (entry.xy[0] !== '.' ? entry.xy[0] : entry.xy[1] ?? 'M');
}

const LETTER_LABEL: Record<string, string> = {
  M: '修改', A: '新增', D: '删除', R: '重命名', C: '复制', U: '冲突', '?': '未跟踪',
};

function fileBasename(path: string): { base: string; dir: string } {
  const slash = path.lastIndexOf('/');
  return slash >= 0
    ? { base: path.slice(slash + 1), dir: `${path.slice(0, slash + 1)} ` }
    : { base: path, dir: '' };
}

export function ChangeRow({ entry, column, diffSource, expanded, onToggle, fontSize }: {
  entry: GitFileEntry; column: 'staged' | 'unstaged'; diffSource: string | null;
  expanded: boolean; onToggle: () => void; fontSize: number;
}) {
  const [diffState, setDiffState] = useState<DiffLoadState | null>(null);
  const letter = letterOf(entry, column);
  const { base, dir } = fileBasename(entry.path);
  const expandable = diffSource !== null;
  // fetchedRef 做展开期一次性加载（结果即缓存）；失败时复位以便再次展开重试。
  // 不做 abort——折叠再展开的竞态会让进行中的 fetch 永远落不了盘。
  const fetchedRef = useRef(false);

  useEffect(() => {
    if (!expanded || fetchedRef.current || diffSource === null) return;
    fetchedRef.current = true;
    setDiffState({ loading: true, text: null, truncated: false, binary: false });
    void gitApi.diff(entry.path, diffSource)
      .then((result) => {
        setDiffState({
          loading: false,
          text: result?.diff ?? '',
          truncated: result?.truncated ?? false,
          binary: result?.binary ?? false,
        });
      })
      .catch(() => {
        fetchedRef.current = false;
        setDiffState({ loading: false, text: null, truncated: false, binary: false });
      });
  }, [expanded, diffSource, entry.path]);

  return <div className={`git-change-card${expanded ? ' expanded' : ''}`}>
    <button type="button" className="git-change-row" aria-expanded={expandable ? expanded : undefined}
      aria-label={`${entry.path}${expandable ? '' : '（无 diff）'}`}
      onClick={() => { if (expandable) onToggle(); }}>
      <span className="git-change-chevron">{expandable ? (expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />) : null}</span>
      <FileCode2 size={12} className="review-file-icon" />
      <span className="review-file-name"><strong>{base}</strong>{dir && <span className="review-file-dir">{dir}</span>}</span>
      {entry.origPath && <span className="git-change-orig" title={`重命名自 ${entry.origPath}`}>← {entry.origPath}</span>}
      <em className={`git-letter git-letter-${letter === '?' ? 'untracked' : letter.toLowerCase()}`} title={LETTER_LABEL[letter] ?? letter}>{letter}</em>
      {(entry.added != null || entry.removed != null) && <span className="tool-call-delta">
        <em className="add">+{entry.added ?? 0}</em><em className="del">−{entry.removed ?? 0}</em>
      </span>}
    </button>
    {expanded && <div className="git-change-diff">
      {diffState?.loading && <p className="workspace-status" role="status">正在加载 diff…</p>}
      {diffState && !diffState.loading && diffState.text !== null && diffState.text.length > 0 && <>
        <GitDiffView diff={diffState.text} fontSize={fontSize} />
        {diffState.truncated && <p className="git-diff-truncated">diff 过大已截断（512KB 上限）</p>}
      </>}
      {diffState && !diffState.loading && (diffState.text === null || diffState.text.length === 0) &&
        <p className="git-diff-empty">{diffState.binary ? '二进制文件，无文本 diff' : '无 diff 内容'}</p>}
    </div>}
  </div>;
}

function Section({ title, entries, column, diffSource, expandedPath, onToggle, fontSize }: {
  title: string; entries: GitFileEntry[]; column: 'staged' | 'unstaged'; diffSource: string | null;
  expandedPath: string | null; onToggle: (path: string) => void; fontSize: number;
}) {
  if (entries.length === 0) return null;
  const visible = entries.slice(0, ROW_CAP);
  return <section className="git-section">
    <h4 className="git-section-title">{title}<span className="git-section-count">{entries.length}</span></h4>
    {visible.map((entry) => <ChangeRow key={`${column}:${entry.path}`} entry={entry} column={column}
      diffSource={entry.xy === '?' ? null : diffSource}
      expanded={expandedPath === `${column}:${entry.path}`}
      onToggle={() => onToggle(`${column}:${entry.path}`)} fontSize={fontSize} />)}
    {entries.length > ROW_CAP && <p className="git-rows-truncated">仅显示前 {ROW_CAP} 项（共 {entries.length} 项）</p>}
  </section>;
}

export default function GitPane({ git, fontSize }: { git: GitRepoState; fontSize: number }) {
  const { summary, status, loading, refresh } = git;
  const [source, setSource] = useState<SourceKey>('unstaged');
  const [expandedPath, setExpandedPath] = useState<string | null>(null);
  const [branchFiles, setBranchFiles] = useState<GitFileEntry[] | null>(null);
  const [branchLoading, setBranchLoading] = useState(false);

  const upstream = summary?.upstream ?? null;
  const branchSource = upstream ? `branch:${upstream}` : null;

  useEffect(() => {
    if (source !== 'branch' || !branchSource) return;
    let cancelled = false;
    setBranchLoading(true);
    void gitApi.filesOf(branchSource)
      .then((files) => { if (!cancelled) setBranchFiles(files ?? []); })
      .catch(() => { if (!cancelled) setBranchFiles([]); })
      .finally(() => { if (!cancelled) setBranchLoading(false); });
    return () => { cancelled = true; };
  }, [source, branchSource, summary]);

  const toggle = useCallback((key: string) => {
    setExpandedPath((current) => (current === key ? null : key));
  }, []);

  const emptyCopy = useMemo(() => {
    if (source === 'staged') return '没有已暂存的更改';
    if (source === 'branch') return '与上游分支没有差异';
    return '工作区干净，没有未暂存的更改';
  }, [source]);

  if (!summary) {
    return <div className="git-pane"><div className="editor-empty" role="status">正在读取 git 状态…</div></div>;
  }
  if (summary.gitMissing) {
    return <div className="git-pane"><div className="editor-empty"><GitBranch size={34} strokeWidth={1} />
      <h3>未检测到 git</h3><p>系统中找不到 git 可执行程序，安装后重启应用。</p></div></div>;
  }
  if (!summary.repo) {
    return <div className="git-pane"><div className="editor-empty"><GitBranch size={34} strokeWidth={1} />
      <h3>非 git 仓库</h3><p>当前工作区不是 git 仓库，git 面板不可用。</p></div></div>;
  }

  const sections: { title: string; entries: GitFileEntry[]; column: 'staged' | 'unstaged'; diffSource: string | null }[] = [];
  if (source === 'unstaged' && status) {
    sections.push(
      { title: '冲突', entries: status.conflicted, column: 'unstaged', diffSource: 'unstaged' },
      { title: '未暂存的更改', entries: status.unstaged, column: 'unstaged', diffSource: 'unstaged' },
      { title: '未跟踪的文件', entries: status.untracked, column: 'unstaged', diffSource: null },
    );
  } else if (source === 'staged' && status) {
    sections.push({ title: '已暂存的更改', entries: status.staged, column: 'staged', diffSource: 'staged' });
  } else if (source === 'branch' && branchFiles) {
    sections.push({ title: `与 ${upstream} 的差异`, entries: branchFiles, column: 'unstaged', diffSource: branchSource });
  }

  const totalRows = sections.reduce((sum, section) => sum + section.entries.length, 0);

  return <div className="git-pane">
    <div className="git-pane-header">
      <span className="git-branch-badge" title={upstream ? `上游 ${upstream}` : '无上游分支'}>
        <GitBranch size={12} />{summary.branch ?? 'HEAD 游离'}
        {(summary.ahead > 0 || summary.behind > 0) && <em className="git-ahead-behind">
          {summary.ahead > 0 && `↑${summary.ahead}`}{summary.behind > 0 && `↓${summary.behind}`}
        </em>}
      </span>
      <span className="git-pane-spacer" />
      <div className="git-source-switch" role="tablist" aria-label="git 来源">
        {(['unstaged', 'staged'] as const).map((key) => <button key={key} type="button" role="tab" aria-selected={source === key}
          className={`git-source-tab${source === key ? ' active' : ''}`} onClick={() => setSource(key)}>
          {key === 'unstaged' ? '未暂存' : '已暂存'}</button>)}
        {branchSource && <button type="button" role="tab" aria-selected={source === 'branch'}
          className={`git-source-tab${source === 'branch' ? ' active' : ''}`} onClick={() => setSource('branch')}>分支对比</button>}
        <button type="button" role="tab" aria-selected={source === 'log'}
          className={`git-source-tab${source === 'log' ? ' active' : ''}`} onClick={() => setSource('log')}>历史</button>
      </div>
      <button type="button" className="icon-button" aria-label="刷新 git 状态" disabled={loading} onClick={refresh}><RefreshCw size={12} /></button>
    </div>
    <div className="git-pane-body">
      {/* GitCommitLog 保持挂载（hidden 切换）——卸载会丢失分页与展开状态。 */}
      <div hidden={source !== 'log'}><GitCommitLog fontSize={fontSize} active={source === 'log'} /></div>
      {source !== 'log' && <>
        {source === 'branch' && branchLoading && <p className="workspace-status" role="status">正在计算分支差异…</p>}
        {totalRows === 0 && !(source === 'branch' && branchLoading)
          ? <p className="review-files-empty">{emptyCopy}</p>
          : sections.map((section) => <Section key={section.title} title={section.title} entries={section.entries}
            column={section.column} diffSource={section.diffSource}
            expandedPath={expandedPath} onToggle={toggle} fontSize={fontSize} />)}
      </>}
    </div>
  </div>;
}

export type { GitStatus, GitSummary };
