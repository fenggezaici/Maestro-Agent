import { useMemo } from 'react';
import * as Tabs from '@radix-ui/react-tabs';
import { Files, GitCompareArrows } from 'lucide-react';
import DiffPanel, { type DiffViewState } from './DiffPanel';
import ExplorerPanel, { type ExplorerViewState } from './ExplorerPanel';
import { mockExplorerFiles } from './workbenchMock';
import type { PlanCard } from '../ipc';
import LiveExplorerPanel from './LiveExplorerPanel';
import type { WorkspaceController } from '../useWorkspace';

export default function RightPanel({ tab, onTabChange, plans, selectedId, onSelect, diffView, onDiffViewChange, explorerView, onExplorerViewChange, scope, workspaceName, fontSize, fontFamily, preview, workspace }: {
  tab: 'review' | 'explorer'; onTabChange: (tab: 'review' | 'explorer') => void;
  plans: PlanCard[]; selectedId: number | null; onSelect: (id: number) => void;
  diffView: DiffViewState; onDiffViewChange: (view: DiffViewState) => void;
  explorerView: ExplorerViewState; onExplorerViewChange: (view: ExplorerViewState) => void;
  scope: string; workspaceName: string; fontSize: number; fontFamily: string;
  preview: boolean; workspace: WorkspaceController;
}) {
  const plan = plans.find((item) => item.id === selectedId) ?? plans.at(-1);
  const files = useMemo(() => {
    if (!preview) return [];
    const items = new Map(mockExplorerFiles.map((file) => [file.path, { ...file, id: `${scope}:mock:${file.id}` }]));
    for (const diff of plan?.draft.diffs ?? []) items.set(diff.path, { id: `${scope}:proposal:${plan!.id}:${diff.path}`, path: diff.path, content: diff.after, source: 'proposal' });
    return [...items.values()];
  }, [scope, plan, preview]);
  return <aside aria-label="右侧工作区" className="right-pane">
    <Tabs.Root value={tab} onValueChange={(value) => onTabChange(value as 'review' | 'explorer')} className="right-tabs">
      <Tabs.List aria-label="右侧视图"><Tabs.Trigger value="review" aria-label="Review"><GitCompareArrows size={14} />Review</Tabs.Trigger><Tabs.Trigger value="explorer" aria-label="Explorer"><Files size={14} />Explorer<span className="mock-badge">{preview ? 'Mock' : 'Local'}</span></Tabs.Trigger></Tabs.List>
    </Tabs.Root>
    <div className="right-panel-body">
      {tab === 'review' ? <DiffPanel plans={plans} selectedId={selectedId} onSelect={onSelect} view={diffView} onViewChange={onDiffViewChange} workspaceName={workspaceName} fontSize={fontSize} fontFamily={fontFamily} />
        : preview ? <ExplorerPanel files={files} scope={scope} view={explorerView} onViewChange={onExplorerViewChange} fontSize={fontSize} fontFamily={fontFamily} />
          : <LiveExplorerPanel workspace={workspace} view={explorerView} onViewChange={onExplorerViewChange} fontSize={fontSize} fontFamily={fontFamily} />}
    </div>
  </aside>;
}
