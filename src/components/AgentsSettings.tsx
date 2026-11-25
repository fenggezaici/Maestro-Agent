import { useEffect, useState } from 'react';
import { Bot } from 'lucide-react';
import { agentDefaultsApi, listAgentsApi, saveAgentApi, saveAgentDefaultsApi, type AgentDefaults, type AgentDefinition, type AgentSource } from '../api/agents';
import { workspaceApi } from '../api/workspace';

const READ_ONLY_TOOLS = ['read_file', 'list_dir', 'load_skill'];
const SOURCE_LABEL: Record<AgentSource, string> = { builtin: '内置', global: '全局', project: '项目' };
const SOURCE_ORDER: AgentSource[] = ['builtin', 'global', 'project'];

const BUILTIN: AgentDefinition[] = [
  { name: 'explore', description: '只读代码调研：并行查找文件、证据路径与风险', category: '研究', mode: 'subagent', prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#8B5CF6', source: 'builtin' },
  { name: 'general', description: '只读分析总结：整理多来源信息并给出结论', category: '研究', mode: 'subagent', prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#22D3EE', source: 'builtin' },
];

type Draft = { agent: AgentDefinition; scope: 'global' | 'project'; isNew: boolean };

/// 扩展面板「编辑 / 新建」的深链目标：name 为 null 表示新建草稿。
export interface AgentFocus {
  name: string | null;
  tick: number;
}

const newDraft = (): Draft => ({
  agent: { name: '', description: '', category: '研究', mode: 'subagent', prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: null, toolBudget: null, hidden: false, color: '#8B5CF6', source: 'global' },
  scope: 'global',
  isNew: true,
});

const EMPTY_DEFAULTS: AgentDefaults = { model: null, steps: null, toolBudget: null, maxParallel: null, maxTotal: null };

export default function AgentsSettings({ preview, focus }: { preview: boolean; focus?: AgentFocus | null }) {
  const [agents, setAgents] = useState<AgentDefinition[]>(preview ? BUILTIN : []);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hasWorkspace, setHasWorkspace] = useState(false);
  const [defaults, setDefaults] = useState<AgentDefaults>(EMPTY_DEFAULTS);
  const [defaultsNote, setDefaultsNote] = useState<string | null>(null);

  useEffect(() => {
    if (preview) return;
    void listAgentsApi('all').then((list) => { setAgents(list); setError(null); }).catch((reason) => setError(String(reason)));
    void agentDefaultsApi().then(setDefaults).catch((reason) => setError(String(reason)));
    void workspaceApi.info().then((info) => setHasWorkspace(Boolean(info.path))).catch(() => setHasWorkspace(false));
  }, [preview]);

  const patchDefaults = (value: Partial<AgentDefaults>) => { setDefaults((current) => ({ ...current, ...value })); setDefaultsNote(null); };
  const numberOrNull = (raw: string, min = 1, max = 64) => {
    if (raw.trim() === '') return null;
    const value = Number(raw);
    return Number.isFinite(value) ? Math.min(max, Math.max(min, Math.trunc(value))) : null;
  };
  const saveDefaults = () => {
    if (preview) { setDefaultsNote('预览模式不会写入'); return; }
    void saveAgentDefaultsApi(defaults).then(() => setDefaultsNote('已保存全局默认')).catch((reason) => setError(String(reason)));
  };

  const refresh = () => listAgentsApi('all').then((list) => { setAgents(list); setError(null); }).catch((reason) => setError(String(reason)));

  // 深链：扩展面板点「编辑 / 新建」直达此处；列表可能尚未加载完，依赖 agents 再解析。
  useEffect(() => {
    if (!focus) return;
    if (focus.name === null) { setDraft(newDraft()); return; }
    const target = agents.find((agent) => agent.name === focus.name);
    if (target) setDraft({ agent: target, scope: target.source === 'project' ? 'project' : 'global', isNew: false });
  }, [focus, agents]);

  const select = (name: string) => {
    if (!name) { setDraft(null); return; }
    const target = agents.find((agent) => agent.name === name);
    if (target) setDraft({ agent: target, scope: target.source === 'project' ? 'project' : 'global', isNew: false });
  };

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

  const note = !draft || draft.isNew ? null
    : draft.agent.source === 'builtin' ? '保存会写入全局覆盖文件，内置定义本身不变；删除覆盖后恢复内置。'
    : `保存到${SOURCE_LABEL[draft.agent.source ?? 'global']}作用域，名称不可修改。`;

  return <div className="agents-settings">
    <p className="agent-editor-note">管理安装、搜索与批量导入在「扩展 → 子智能体」中进行；此处负责全局默认与单个子智能体的配置。</p>
    <h4 className="agents-section-title">全局默认</h4>
    <div className="agent-editor">
      <label>默认模型（留空跟随会话）<input value={defaults.model ?? ''} onChange={(event) => patchDefaults({ model: event.target.value.trim() || null })} placeholder="跟随会话模型" /></label>
      <label>默认轮次上限<input type="number" min={1} max={64} value={defaults.steps ?? ''} placeholder="8" onChange={(event) => patchDefaults({ steps: numberOrNull(event.target.value) })} /></label>
      <label>默认工具预算（次）<input type="number" min={1} max={500} value={defaults.toolBudget ?? ''} placeholder="100" onChange={(event) => patchDefaults({ toolBudget: numberOrNull(event.target.value, 1, 500) })} /></label>
      <label>每轮并行上限<input type="number" min={1} max={4} value={defaults.maxParallel ?? ''} placeholder="2" onChange={(event) => patchDefaults({ maxParallel: numberOrNull(event.target.value, 1, 4) })} /></label>
      <label>单 run 总额度<input type="number" min={1} max={16} value={defaults.maxTotal ?? ''} placeholder="8" onChange={(event) => patchDefaults({ maxTotal: numberOrNull(event.target.value, 1, 16) })} /></label>
      <p className="agent-editor-note agent-editor-wide">留空回退内置默认；每个子智能体的字段留空则继承这里的全局默认（模型最终回退到会话模型）。</p>
      <div className="agent-editor-actions">{defaultsNote && <span className="agent-editor-note">{defaultsNote}</span>}<button type="button" className="button-approve" onClick={saveDefaults}>保存全局默认</button></div>
    </div>
    <h4 className="agents-section-title">单个子智能体</h4>
    <label className="agents-picker">配置对象
      <select aria-label="选择要配置的子智能体" value={draft && !draft.isNew ? draft.agent.name : ''} onChange={(event) => select(event.target.value)}>
        <option value="">选择要配置的子智能体…</option>
        {SOURCE_ORDER.map((source) => {
          const group = agents.filter((agent) => (agent.source ?? 'global') === source);
          return group.length > 0 ? <optgroup key={source} label={SOURCE_LABEL[source]}>{group.map((agent) => <option key={agent.name} value={agent.name}>{agent.name}{agent.hidden ? '（已禁用）' : ''}</option>)}</optgroup> : null;
        })}
      </select>
    </label>
    {error && <p role="alert" className="settings-form-error">{error}</p>}
    {draft && <div className="agent-editor">
      <label>名称<input value={draft.agent.name} disabled={!draft.isNew} onChange={(event) => patch({ name: event.target.value })} placeholder="reviewer" /></label>
      <label>描述<input value={draft.agent.description} onChange={(event) => patch({ description: event.target.value })} /></label>
      <label>分类<input value={draft.agent.category ?? ''} onChange={(event) => patch({ category: event.target.value })} /></label>
      <label>模型（留空继承全局默认/会话）<input value={draft.agent.model ?? ''} onChange={(event) => patch({ model: event.target.value || null })} /></label>
      <label>轮次上限（留空继承）<input type="number" min={1} max={64} value={draft.agent.steps ?? ''} placeholder={String(defaults.steps ?? 8)} onChange={(event) => patch({ steps: numberOrNull(event.target.value) })} /></label>
      <label>工具预算（次，留空继承）<input type="number" min={1} max={500} value={draft.agent.toolBudget ?? ''} placeholder={String(defaults.toolBudget ?? 100)} onChange={(event) => patch({ toolBudget: numberOrNull(event.target.value, 1, 500) })} /></label>
      <label>作用域<select value={draft.scope} disabled={!draft.isNew} onChange={(event) => setDraft({ ...draft, scope: event.target.value as Draft['scope'] })}><option value="global">全局 ~/.maestro/agents</option><option value="project" disabled={!hasWorkspace}>项目 &lt;workspace&gt;/.maestro/agents</option></select></label>
      <label className="agent-editor-wide agent-disabled-toggle"><input type="checkbox" checked={draft.agent.hidden} onChange={(event) => patch({ hidden: event.target.checked })} />禁用（hidden：从派发枚举与沙盘物料中隐藏，定义保留）</label>
      <div className="agent-editor-wide agent-tools"><span>只读工具</span>{READ_ONLY_TOOLS.map((tool) => <label key={tool}><input type="checkbox" checked={draft.agent.tools.includes(tool)} onChange={() => toggleTool(tool)} />{tool}</label>)}<small>全不勾选时使用全部只读工具</small></div>
      <label className="agent-editor-wide">Prompt<textarea rows={5} value={draft.agent.prompt} onChange={(event) => patch({ prompt: event.target.value })} /></label>
      {note && <p className="agent-editor-note agent-editor-wide">{note}</p>}
      <div className="agent-editor-actions"><button type="button" className="button-ghost" onClick={() => setDraft(null)}>取消</button><button type="button" className="button-approve" onClick={save}>保存</button></div>
    </div>}
    {!draft && <div className="agents-empty"><Bot size={26} /><strong>选择要配置的子智能体</strong><p>从上方下拉选择要配置的子智能体；新建与批量导入在「扩展 → 子智能体」中进行。</p></div>}
  </div>;
}
