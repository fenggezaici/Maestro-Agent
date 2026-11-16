import { useEffect, useMemo, useState } from 'react';
import { Bot, Plus, Search, Trash2 } from 'lucide-react';
import { deleteAgentApi, listAgentsApi, saveAgentApi, type AgentDefinition, type AgentSource } from '../api/agents';
import { workspaceApi } from '../api/workspace';
import ConfirmDialog from './ConfirmDialog';

const READ_ONLY_TOOLS = ['read_file', 'list_dir', 'load_skill'];
const SOURCE_LABEL: Record<AgentSource, string> = { builtin: '内置', global: '全局', project: '项目' };

const BUILTIN: AgentDefinition[] = [
  { name: 'explore', description: '只读代码调研：并行查找文件、证据路径与风险', category: '研究', mode: 'subagent', prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#8B5CF6', source: 'builtin' },
  { name: 'general', description: '只读分析总结：整理多来源信息并给出结论', category: '研究', mode: 'subagent', prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#22D3EE', source: 'builtin' },
];

type Draft = { agent: AgentDefinition; scope: 'global' | 'project'; isNew: boolean };

export default function AgentsSettings({ preview }: { preview: boolean }) {
  const [agents, setAgents] = useState<AgentDefinition[]>(preview ? BUILTIN : []);
  const [query, setQuery] = useState('');
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<AgentDefinition | null>(null);
  const [hasWorkspace, setHasWorkspace] = useState(false);

  useEffect(() => {
    if (preview) return;
    void listAgentsApi('all').then((list) => { setAgents(list); setError(null); }).catch((reason) => setError(String(reason)));
    void workspaceApi.info().then((info) => setHasWorkspace(Boolean(info.path))).catch(() => setHasWorkspace(false));
  }, [preview]);

  const refresh = () => listAgentsApi('all').then((list) => { setAgents(list); setError(null); }).catch((reason) => setError(String(reason)));
  const visible = useMemo(() => agents.filter((agent) => `${agent.name} ${agent.description}`.toLowerCase().includes(query.trim().toLowerCase())), [agents, query]);
  const patch = (value: Partial<AgentDefinition>) => setDraft((current) => current ? { ...current, agent: { ...current.agent, ...value } } : current);
  const toggleTool = (tool: string) => setDraft((current) => current ? {
    ...current,
    agent: { ...current.agent, tools: current.agent.tools.includes(tool) ? current.agent.tools.filter((item) => item !== tool) : [...current.agent.tools, tool] },
  } : current);

  const save = () => {
    if (!draft || !draft.agent.name.trim()) return;
    if (preview) {
      const saved = { ...draft.agent, source: draft.scope };
      setAgents((current) => [...current.filter((agent) => agent.name !== saved.name), saved]);
      setDraft(null);
      return;
    }
    void saveAgentApi(draft.agent, draft.scope).then(refresh).then(() => setDraft(null)).catch((reason) => setError(String(reason)));
  };

  const remove = () => {
    if (!deleting) return;
    const scope = deleting.source === 'project' ? 'project' : 'global';
    if (preview) { setAgents((current) => current.filter((agent) => agent.name !== deleting.name)); setDeleting(null); return; }
    void deleteAgentApi(deleting.name, scope).then(refresh).then(() => setDeleting(null)).catch((reason) => setError(String(reason)));
  };

  const note = !draft || draft.isNew ? null
    : draft.agent.source === 'builtin' ? '保存会写入全局覆盖文件，内置定义本身不变；删除覆盖后恢复内置。'
    : `保存到${SOURCE_LABEL[draft.agent.source ?? 'global']}作用域，名称不可修改。`;

  return <div className="agents-settings">
    <div className="agents-toolbar"><label className="agents-search"><Search size={13} /><input aria-label="搜索子智能体" placeholder="搜索子智能体…" value={query} onChange={(event) => setQuery(event.target.value)} /></label><span className="agents-count">子智能体 {visible.length}</span><button type="button" className="button-ghost" onClick={() => setDraft({ agent: { name: '', description: '', category: '研究', mode: 'subagent', prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#8B5CF6', source: 'global' }, scope: 'global', isNew: true })}><Plus size={12} />新建</button></div>
    {error && <p role="alert" className="settings-form-error">{error}</p>}
    {draft && <div className="agent-editor">
      <label>名称<input value={draft.agent.name} disabled={!draft.isNew} onChange={(event) => patch({ name: event.target.value })} placeholder="reviewer" /></label>
      <label>描述<input value={draft.agent.description} onChange={(event) => patch({ description: event.target.value })} /></label>
      <label>分类<input value={draft.agent.category ?? ''} onChange={(event) => patch({ category: event.target.value })} /></label>
      <label>模型（留空继承默认）<input value={draft.agent.model ?? ''} onChange={(event) => patch({ model: event.target.value || null })} /></label>
      <label>轮次上限<input type="number" value={draft.agent.steps ?? 8} onChange={(event) => patch({ steps: Number(event.target.value) || 8 })} /></label>
      <label>作用域<select value={draft.scope} disabled={!draft.isNew} onChange={(event) => setDraft({ ...draft, scope: event.target.value as Draft['scope'] })}><option value="global">全局 ~/.maestro/agents</option><option value="project" disabled={!hasWorkspace}>项目 &lt;workspace&gt;/.maestro/agents</option></select></label>
      <div className="agent-editor-wide agent-tools"><span>只读工具</span>{READ_ONLY_TOOLS.map((tool) => <label key={tool}><input type="checkbox" checked={draft.agent.tools.includes(tool)} onChange={() => toggleTool(tool)} />{tool}</label>)}<small>全不勾选时使用全部只读工具</small></div>
      <label className="agent-editor-wide">Prompt<textarea rows={5} value={draft.agent.prompt} onChange={(event) => patch({ prompt: event.target.value })} /></label>
      {note && <p className="agent-editor-note agent-editor-wide">{note}</p>}
      <div className="agent-editor-actions"><button type="button" className="button-ghost" onClick={() => setDraft(null)}>取消</button><button type="button" className="button-approve" onClick={save}>保存</button></div>
    </div>}
    <div className="agents-list">{visible.map((agent) => <div className="agent-card" key={`${agent.source ?? 'global'}:${agent.name}`}>
      <span className="agent-icon" style={{ color: agent.color ?? undefined }}><Bot size={15} /></span>
      <div><strong>{agent.name}</strong><span className="agent-source" data-source={agent.source ?? 'global'}>{SOURCE_LABEL[agent.source ?? 'global']}</span><p>{agent.description || '（无描述）'}</p><small>{agent.tools.length} 个工具 · {agent.model ?? '继承默认'} · {agent.steps ?? 8} 轮</small></div>
      <button type="button" className="button-ghost" onClick={() => setDraft({ agent, scope: agent.source === 'project' ? 'project' : 'global', isNew: false })}>编辑</button>
      <button type="button" className="icon-button" aria-label={`删除 ${agent.name}`} disabled={agent.source === 'builtin'} title={agent.source === 'builtin' ? '内置子智能体不可删除；可编辑为覆盖版本' : undefined} onClick={() => setDeleting(agent)}><Trash2 size={13} /></button>
    </div>)}</div>
    {visible.length === 0 && <div className="agents-empty"><Bot size={26} /><strong>没有找到子智能体</strong><p>新建一个只读研究 agent，供 spawn_subagent 使用。保存位置：~/.maestro/agents 或 &lt;workspace&gt;/.maestro/agents。</p></div>}
    <ConfirmDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)} title="删除子智能体" description={`删除 ${deleting?.name ?? ''}？`} onConfirm={remove} />
  </div>;
}
