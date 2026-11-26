import { useCallback, useEffect, useRef, useState } from 'react';
import { gitApi, type GitStatus, type GitSummary } from '../api/git';

export interface GitRepoState {
  summary: GitSummary | null;
  status: GitStatus | null;
  loading: boolean;
  refresh: () => void;
}

/// git 只读感知数据：summary + status 并行拉取。enabled=false（preview/未选工作区）
/// 时不发任何 IPC。tick 来自 kernel 的 workspaceTick——agent 写文件/跑命令后
/// 合并刷新，与文件树同一节拍。
export function useGitRepo(enabled: boolean, tick: number): GitRepoState {
  const [summary, setSummary] = useState<GitSummary | null>(null);
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);

  const refresh = useCallback(() => {
    if (!enabled) return;
    const gen = ++generation.current;
    setLoading(true);
    void Promise.all([gitApi.summary(), gitApi.status()])
      .then(([nextSummary, nextStatus]) => {
        if (generation.current !== gen) return;
        setSummary(nextSummary);
        setStatus(nextStatus);
      })
      .catch(() => {
        // IPC 失败（如未选工作区）保持旧数据——git 面板不是关键路径，不弹错。
      })
      .finally(() => {
        if (generation.current === gen) setLoading(false);
      });
  }, [enabled]);

  useEffect(() => {
    if (!enabled) {
      setSummary(null);
      setStatus(null);
      return;
    }
    refresh();
  }, [enabled, refresh]);

  useEffect(() => {
    if (!enabled || tick === 0) return;
    const timer = window.setTimeout(refresh, 400);
    return () => window.clearTimeout(timer);
  }, [enabled, tick, refresh]);

  return { summary, status, loading, refresh };
}
