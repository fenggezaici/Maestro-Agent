import { useEffect, useState } from 'react';
import { useT } from '../i18n';
import * as Tabs from '@radix-ui/react-tabs';
import * as Switch from '@radix-ui/react-switch';
import { Bot, ExternalLink, FolderInput, GitBranch, PlugZap, Plus, Puzzle, Search, Sparkles, Trash2, X } from 'lucide-react';
import type { ExtensionsController } from '../hooks/useExtensions';
import type { McpServerView, RecommendedItem, SkillView } from '../api/extensions';
import { agentRef, type AgentDefinition, type AgentSource } from '../api/agents';
import { formatEnvText, parseEnvText, parseHeadersText } from '../lib/envText';
import ConfirmDialog from './ConfirmDialog';
import FormDialog from './ui/FormDialog';

function SkillList({ c }: { c: ExtensionsController }) {
  const [dialog, setDialog] = useState<'create' | 'local' | 'github' | null>(null);
  const [deleting, setDeleting] = useState<SkillView | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  return <section aria-label="技能列表" className="extension-section">
    <div className="extension-section-heading">
      <h4><Sparkles size={14} />技能（Skills）</h4>
      <div className="extension-actions">
        <button type="button" className="button-ghost" onClick={() => setDialog('create')}><Plus size={13} />新建技能</button>
        <button type="button" className="button-ghost" onClick={() => setDialog('local')}><FolderInput size={13} />本地导入</button>
        <button type="button" className="button-ghost" onClick={() => setDialog('github')}><GitBranch size={13} />GitHub 导入</button>
      </div>
    </div>
    {c.skills.length === 0 && <p className="sidebar-empty">{c.loading ? '加载中…' : '暂无技能。新建或从发现页安装。'}</p>}
    {deleteError && <p role="alert" className="settings-form-error">{deleteError}</p>}
    {c.importSummary && <p className="import-summary">{c.importSummary}</p>}
    {c.skills.map((skill) => <article className="extension-card" key={`${skill.source}:${skill.name}`}>
      <span className="extension-icon"><Sparkles size={18} /></span>
      <div className="extension-description"><strong>{skill.name}</strong><p>{skill.description || '（无描述）'}</p>
        <span className="extension-state">{skill.source === 'project' ? '项目级' : '全局'} · {skill.path}</span></div>
      <button type="button" className="icon-button" aria-label={`删除技能 ${skill.name}`} disabled={c.pending === `delete-skill-${skill.name}`}
        onClick={() => setDeleting(skill)}><Trash2 size={13} /></button>
    </article>)}
    <FormDialog open={dialog === 'create'} onOpenChange={(o) => !o && setDialog(null)} title="新建技能"
      hint="生成标准 SKILL.md 模板；项目级需要先选择工作区。" submitLabel="创建"
      fields={[
        { id: 'name', label: '名称（kebab-case）', placeholder: 'my-skill' },
        { id: 'description', label: '描述', placeholder: '这个技能做什么？', textarea: true },
        { id: 'scope', label: '作用域', options: [
          { value: 'global', label: '全局 ~/.maestro/skills' },
          { value: 'project', label: '项目 <workspace>/.maestro/skills', disabled: !c.hasWorkspace },
        ] },
      ]}
      onSubmit={(v) => c.createSkill(v.name, v.description, v.scope === 'project' ? 'project' : 'global')} />
    <FormDialog open={dialog === 'local'} onOpenChange={(o) => !o && setDialog(null)} title="从本地导入技能"
      hint="选择 SKILL.md 文件、单个技能目录，或含多个技能目录的父目录（批量导入）。" submitLabel="导入" canBrowse={!!c.real}
      fields={[{ id: 'path', label: '本地路径', placeholder: 'E:\\skills\\my-skill', browse: 'directory' }]}
      onSubmit={(v) => c.importSkill(v.path)} />
    <FormDialog open={dialog === 'github'} onOpenChange={(o) => !o && setDialog(null)} title="从 GitHub 导入技能"
      hint="克隆仓库并递归导入其中全部 SKILL.md。" submitLabel="导入"
      fields={[{ id: 'repo', label: '仓库地址', placeholder: 'https://github.com/owner/repo' }]}
      onSubmit={(v) => c.importSkillGithub(v.repo)} />
    <ConfirmDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)} title="删除技能"
      description={deleting ? `删除技能「${deleting.name}」（${deleting.source === 'project' ? '项目级' : '全局'}）？该技能目录会被移除，不可撤销。` : ''}
      onConfirm={() => {
        if (!deleting) return;
        const target = deleting;
        setDeleting(null);
        void c.deleteSkill(target).then((message) => setDeleteError(message));
      }} />
  </section>;
}

function McpList({ c }: { c: ExtensionsController }) {
  const [dialog, setDialog] = useState<'add' | 'import' | null>(null);
  const [editing, setEditing] = useState<McpServerView | null>(null);
  const open = dialog === 'add' || editing !== null;
  return <section aria-label="MCP 服务列表" className="extension-section">
    <div className="extension-section-heading">
      <h4><PlugZap size={14} />MCP 服务</h4>
      <div className="extension-actions">
        <button type="button" className="button-ghost" onClick={() => setDialog('add')}><Plus size={13} />添加配置</button>
        <button type="button" className="button-ghost" onClick={() => setDialog('import')}><FolderInput size={13} />导入 JSON</button>
      </div>
    </div>
    {c.servers.length === 0 && <p className="sidebar-empty">{c.loading ? '加载中…' : '暂无 MCP 服务。添加配置或从发现页安装。'}</p>}
    {c.servers.map((server) => <McpRow key={server.name} c={c} server={server} onEdit={() => setEditing(server)} />)}
    <FormDialog key={editing ? `edit-${editing.name}` : 'new'} open={open}
      onOpenChange={(value) => { if (!value) { setDialog(null); setEditing(null); } }}
      title={editing ? `编辑 MCP 服务 ${editing.name}` : '添加 MCP 服务'}
      hint="写入 ~/.maestro/mcp.json；启用状态会跨重启保留，应用启动时自动拉起已启用的服务。"
      submitLabel={editing ? '保存' : '添加'}
      initialValues={editing ? {
        name: editing.name,
        transport: editing.transport ?? 'stdio',
        command: editing.command, args: editing.args.join(' '), env: formatEnvText(editing.env),
        url: editing.url ?? '', headers: editing.headerKeys.map((key) => `${key}: `).join('\n'),
        timeoutMs: editing.timeoutMs ? String(editing.timeoutMs) : '',
      } : undefined}
      fields={[
        { id: 'name', label: '名称', placeholder: 'filesystem', disabled: Boolean(editing) },
        { id: 'transport', label: '类型', required: false, options: [
          { value: 'stdio', label: 'stdio（本地命令）' },
          { value: 'http', label: 'HTTP（Streamable）' },
          { value: 'sse', label: 'SSE（Server-Sent Events）' },
        ] },
        { id: 'command', label: '启动命令', placeholder: 'npx', when: (v) => (v.transport ?? 'stdio') === 'stdio' },
        { id: 'args', label: '参数（空格或逗号分隔，可选）', placeholder: '-y @modelcontextprotocol/server-filesystem', required: false, when: (v) => (v.transport ?? 'stdio') === 'stdio' },
        { id: 'env', label: '环境变量（JSON 或每行 KEY=VALUE，可选）', placeholder: 'GITHUB_PERSONAL_ACCESS_TOKEN=ghp_xxx', textarea: true, required: false, when: (v) => (v.transport ?? 'stdio') === 'stdio' },
        { id: 'url', label: 'URL', placeholder: 'https://api.githubcopilot.com/mcp/', when: (v) => (v.transport ?? 'stdio') !== 'stdio' },
        { id: 'headers', label: '请求头（JSON 或每行 Name: value，可选）', placeholder: 'Authorization: Bearer ghp_xxx', textarea: true, required: false, when: (v) => (v.transport ?? 'stdio') !== 'stdio' },
        { id: 'timeoutMs', label: '启动超时（毫秒，可选）', placeholder: '默认 stdio 60s / 远程 20s；npx 冷装可调大', required: false },
      ]}
      onSubmit={(v) => {
        const parsed = parseEnvText(v.env ?? '');
        if (parsed.error) return Promise.resolve(parsed.error);
        const headers = parseHeadersText(v.headers ?? '');
        if (headers.error) return Promise.resolve(headers.error);
        const transport = v.transport ?? 'stdio';
        const timeoutText = (v.timeoutMs ?? '').trim();
        const timeoutMs = timeoutText ? Number(timeoutText) : undefined;
        if (timeoutText && (!Number.isFinite(timeoutMs) || timeoutMs === undefined || timeoutMs < 1000 || timeoutMs > 600000)) return Promise.resolve('启动超时需为 1000–600000 毫秒之间的数字');
        return c.addMcpServer(v.name, transport === 'stdio' ? v.command : '', (v.args ?? '').split(/[\s,]+/).filter(Boolean), parsed.env, editing?.trust ?? false, transport, transport === 'stdio' ? undefined : v.url, headers.map, timeoutMs);
      }} />
    <FormDialog open={dialog === 'import'} onOpenChange={(o) => !o && setDialog(null)} title="导入 MCP JSON"
      hint="读取一个含 mcpServers 字段的 JSON 文件并合并到 ~/.maestro/mcp.json。" submitLabel="导入" canBrowse={!!c.real}
      fields={[{ id: 'path', label: 'JSON 文件路径', placeholder: 'E:\\config\\mcp.json', browse: 'file' }]}
      onSubmit={(v) => c.importMcpJson(v.path)} />
  </section>;
}

function McpRow({ c, server, onEdit }: { c: ExtensionsController; server: McpServerView; onEdit: () => void }) {
  const busy = c.pending === `mcp-${server.name}`;
  const envKeys = Object.keys(server.env ?? {});
  return <article className="extension-card" key={server.name}>
    <span className={`extension-status-dot ${server.running ? 'running' : server.error ? 'error' : ''}`} aria-hidden="true" />
    <div className="extension-description">
      <strong>{server.name}</strong>
      <p><code>{server.transport === 'stdio' ? `${server.command} ${server.args.join(' ')}` : `${server.transport} ${server.url ?? ''}`}</code></p>
      {envKeys.length > 0 && <p className="extension-state">环境变量：{envKeys.join('、')}</p>}
      {server.headerKeys.length > 0 && <p className="extension-state">请求头：{server.headerKeys.join('、')}</p>}
      <span className={`extension-state${!busy && !server.running && server.error ? ' extension-error' : ''}`}>
        {busy ? '处理中…' : server.running ? `运行中 · 已加载 ${server.toolCount} 个工具` : server.error ? server.error : server.disabled ? '已禁用（重启后保持）' : '已停止'}
      </span>
    </div>
    <span className="extension-state" title="只有显式信任的服务才允许其 readOnlyHint 生效">信任只读</span>
    <Switch.Root aria-label={`信任只读声明 ${server.name}`} className="settings-switch" checked={server.trust} disabled={busy}
      onCheckedChange={(trust) => void c.setMcpTrust(server, trust)}><Switch.Thumb /></Switch.Root>
    <button type="button" className="button-ghost" disabled={busy} onClick={onEdit}>编辑</button>
    <Switch.Root aria-label={`启用 ${server.name}`} className="settings-switch" checked={!server.disabled} disabled={busy}
      onCheckedChange={(enable) => void c.toggleMcpServer(server, enable)}><Switch.Thumb /></Switch.Root>
    <button type="button" className="icon-button" aria-label={`删除 ${server.name}`} disabled={busy} onClick={() => void c.removeMcpServer(server.name)}><Trash2 size={13} /></button>
  </article>;
}

const AGENT_SOURCE_LABEL: Record<AgentSource, string> = { builtin: '内置', global: '全局', project: '项目' };

function AgentList({ c, onConfigureAgent }: { c: ExtensionsController; onConfigureAgent?: (name: string | null) => void }) {
  const [dialog, setDialog] = useState<'local' | 'github' | null>(null);
  const [deleting, setDeleting] = useState<AgentDefinition | null>(null);
  const [bulkDelete, setBulkDelete] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<'all' | 'enabled' | 'disabled'>('all');
  // 勾选键 = `${source}:${name}`，与卡片 key 一致；已删项随列表刷新自然脱落。
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const keyOf = (agent: AgentDefinition) => `${agent.source ?? 'global'}:${agent.name}`;
  const needle = query.trim().toLowerCase();
  const visible = c.agents.filter((agent) =>
    (filter === 'all' || (filter === 'disabled') === agent.hidden)
    && `${agent.name} ${agent.description}`.toLowerCase().includes(needle));
  const selectedAgents = c.agents.filter((agent) => selected.has(keyOf(agent)));
  const bulkBusy = c.pending === 'bulk-agents';
  const disabledCount = c.agents.filter((agent) => agent.hidden).length;

  const toggle = (key: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(key)) { next.delete(key); } else { next.add(key); }
    return next;
  });
  const selectVisible = () => setSelected(new Set(visible.map(keyOf)));
  const bulkHidden = (hidden: boolean) => {
    void c.setAgentsHidden(selectedAgents.map(agentRef), hidden).then((message) => {
      if (!message) setSelected(new Set());
      setDeleteError(message);
    });
  };
  const deletable = selectedAgents.filter((agent) => agent.source !== 'builtin');
  const skippedBuiltin = selectedAgents.length - deletable.length;

  return <section aria-label="子智能体列表" className="extension-section">
    <div className="extension-section-heading">
      <h4><Bot size={14} />子智能体（Subagents）</h4>
      <div className="extension-actions">
        <button type="button" className="button-ghost" onClick={() => onConfigureAgent?.(null)}><Plus size={13} />新建</button>
        <button type="button" className="button-ghost" onClick={() => setDialog('local')}><FolderInput size={13} />本地导入</button>
        <button type="button" className="button-ghost" onClick={() => setDialog('github')}><GitBranch size={13} />GitHub 导入</button>
      </div>
    </div>
    <div className="agents-toolbar">
      <label className="agents-search"><Search size={13} /><input aria-label="搜索子智能体" placeholder="搜索子智能体…" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
      <select className="agents-filter" aria-label="按启用状态筛选" value={filter} onChange={(event) => setFilter(event.target.value as typeof filter)}>
        <option value="all">全部（{c.agents.length}）</option>
        <option value="enabled">仅启用</option>
        <option value="disabled">已禁用（{disabledCount}）</option>
      </select>
      <span className="agents-count">子智能体 {visible.length}</span>
    </div>
    {selected.size > 0 && (
      <div className="agents-bulkbar" role="toolbar" aria-label="批量操作">
        <span className="agents-bulkbar-count">已选 {selectedAgents.length}</span>
        <button type="button" className="button-ghost" onClick={selectVisible}>全选当前筛选</button>
        <button type="button" className="button-ghost" disabled={bulkBusy} onClick={() => bulkHidden(true)}>禁用</button>
        <button type="button" className="button-ghost" disabled={bulkBusy} onClick={() => bulkHidden(false)}>启用</button>
        <button type="button" className="button-ghost agents-bulkbar-danger" disabled={bulkBusy || deletable.length === 0}
          title={deletable.length === 0 ? '所选均为内置，不可删除（可禁用）' : undefined}
          onClick={() => setBulkDelete(true)}><Trash2 size={12} />删除</button>
        <button type="button" className="icon-button" aria-label="清除选择" onClick={() => setSelected(new Set())}><X size={13} /></button>
      </div>
    )}
    {c.agents.length === 0 && <p className="sidebar-empty">{c.loading ? '加载中…' : '暂无子智能体。新建或批量导入角色包（如 agency-agents-zh）。'}</p>}
    {c.agents.length > 0 && visible.length === 0 && <p className="sidebar-empty">没有匹配的子智能体。</p>}
    {deleteError && <p role="alert" className="settings-form-error">{deleteError}</p>}
    {c.agentImportSummary && <p className="import-summary">{c.agentImportSummary}</p>}
    <div className="agents-list">{visible.map((agent) => {
      const key = keyOf(agent);
      return <div className={`agent-card${agent.hidden ? ' agent-disabled' : ''}`} key={key}>
        <input type="checkbox" className="agent-check" aria-label={`选择 ${agent.name}`}
          checked={selected.has(key)} onChange={() => toggle(key)} />
        <span className="agent-icon" style={{ color: agent.color ?? undefined }}><Bot size={15} /></span>
        <div><strong>{agent.name}</strong><span className="agent-source" data-source={agent.source ?? 'global'}>{AGENT_SOURCE_LABEL[agent.source ?? 'global']}</span>
          {agent.hidden && <span className="agent-disabled-badge">已禁用</span>}
          <p>{agent.description || '（无描述）'}</p><small>{agent.tools.length} 个工具 · {agent.model ?? '继承默认'} · {agent.steps ?? c.agentDefaults?.steps ?? 8} 轮 · {agent.toolBudget ?? c.agentDefaults?.toolBudget ?? 100} 次预算</small></div>
        <Switch.Root aria-label={`启用 ${agent.name}`} className="settings-switch" checked={!agent.hidden} disabled={bulkBusy}
          title={agent.hidden ? '启用后可被 spawn_subagent 派发' : '禁用后从派发枚举与沙盘物料中隐藏'}
          onCheckedChange={(enable) => void c.setAgentsHidden([agentRef(agent)], !enable).then((message) => setDeleteError(message))}><Switch.Thumb /></Switch.Root>
        <button type="button" className="button-ghost" disabled={!onConfigureAgent} title={onConfigureAgent ? '在「设置 → 子智能体」中配置' : undefined} onClick={() => onConfigureAgent?.(agent.name)}>编辑</button>
        <button type="button" className="icon-button" aria-label={`删除 ${agent.name}`} disabled={agent.source === 'builtin' || c.pending === `delete-agent-${agent.name}`} title={agent.source === 'builtin' ? '内置子智能体不可删除；可禁用' : undefined} onClick={() => setDeleting(agent)}><Trash2 size={13} /></button>
      </div>;
    })}</div>
    <p className="agent-editor-note">「编辑 / 新建」会打开「设置 → 子智能体」完成具体配置；禁用 = hidden，从派发枚举与沙盘物料中隐藏。</p>
    <FormDialog open={dialog === 'local'} onOpenChange={(o) => !o && setDialog(null)} title="从本地目录导入子智能体"
      hint="递归导入目录中的 .md 智能体文件到 ~/.maestro/agents（兼容 agency-agents-zh：文件名作 slug，中文角色名归位描述）；同名跳过。" submitLabel="导入" canBrowse={!!c.real}
      fields={[{ id: 'path', label: '本地目录', placeholder: 'D:\\agents-pack', browse: 'directory' }]}
      onSubmit={(v) => c.importAgentsDir(v.path)} />
    <FormDialog open={dialog === 'github'} onOpenChange={(o) => !o && setDialog(null)} title="从 GitHub 导入子智能体"
      hint="浅克隆仓库并递归导入全部 .md 智能体定义，例如 https://github.com/jnMetaCode/agency-agents-zh。" submitLabel="导入"
      fields={[{ id: 'repo', label: '仓库地址', placeholder: 'https://github.com/owner/repo' }]}
      onSubmit={(v) => c.importAgentsGithub(v.repo)} />
    <ConfirmDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)} title="删除子智能体"
      description={deleting ? `删除 ${deleting.name}（${AGENT_SOURCE_LABEL[deleting.source ?? 'global']}作用域）？对应 .md 定义会被移除，不可撤销。` : ''}
      onConfirm={() => {
        if (!deleting) return;
        const target = deleting;
        setDeleting(null);
        void c.deleteAgent(target).then((message) => setDeleteError(message));
      }} />
    <ConfirmDialog open={bulkDelete} onOpenChange={(open) => !open && setBulkDelete(false)} title={`批量删除 ${deletable.length} 个子智能体`}
      description={`将删除：${deletable.slice(0, 8).map((agent) => agent.name).join('、')}${deletable.length > 8 ? ` 等 ${deletable.length} 项` : ''}。对应 .md 定义会被移除，不可撤销。${skippedBuiltin > 0 ? `（已选中的 ${skippedBuiltin} 个内置项自动跳过）` : ''}`}
      onConfirm={() => {
        const items = deletable.map(agentRef);
        setBulkDelete(false);
        void c.deleteAgents(items).then((message) => {
          if (!message) setSelected(new Set());
          setDeleteError(message);
        });
      }} />
  </section>;
}

function DiscoverGrid({ title, icon, items, c, empty }: { title: string; icon: React.ReactNode; items: RecommendedItem[]; c: ExtensionsController; empty: string }) {
  return <section aria-label={title} className="extension-section">
    <div className="extension-section-heading"><h4>{icon}{title}</h4></div>
    {items.length === 0 && <p className="sidebar-empty">{empty}</p>}
    {items.map((item) => { const installed = c.isInstalled(item); const failure = c.installErrors[item.id];
      return <article className="extension-card discover-card" key={item.id}>
      <div className="extension-description">
        <strong>{item.name}</strong>
        <p>{item.description}</p>
        <a className="extension-link" href={item.sourceUrl} target="_blank" rel="noreferrer">{item.sourceUrl}<ExternalLink size={10} /></a>
        {!!failure && <p className="settings-form-error" role="alert">{failure}</p>}
      </div>
      <button type="button" className="button-approve" aria-label={`安装 ${item.name}`} disabled={installed || c.pending === `install-${item.id}`}
        onClick={async () => { await c.installRecommended(item); }}>{installed ? '已安装' : c.pending === `install-${item.id}` ? '安装中…' : '一键安装'}</button>
    </article>; })}
  </section>;
}

export default function ExtensionsView({ controller: c, onConfigureAgent }: { controller: ExtensionsController; onConfigureAgent?: (name: string | null) => void }) {
  const t = useT();
  return <section aria-label="扩展工作区" className="extensions-view">
    <header className="extensions-heading"><span className="welcome-icon"><Puzzle size={26} strokeWidth={1.2} /></span><div><p className="eyebrow">{t.workspaceCapabilities}</p><h3>{t.extensions}</h3><p>Skills、子智能体与 MCP 的完整生命周期管理</p></div></header>
    {c.loadError && <p role="alert" className="settings-form-error extensions-load-error">部分扩展数据加载失败：{c.loadError}（若涉及 mcp.json，请检查 ~/.maestro/mcp.json 是否为合法 JSON）</p>}
    <Tabs.Root defaultValue="skills" className="extensions-tabs"><Tabs.List aria-label="扩展分类">
      <Tabs.Trigger value="skills">技能</Tabs.Trigger>
      <Tabs.Trigger value="mcp">MCP 服务</Tabs.Trigger>
      <Tabs.Trigger value="agents">子智能体</Tabs.Trigger>
      <Tabs.Trigger value="discover">发现</Tabs.Trigger>
      <Tabs.Trigger value="config">配置</Tabs.Trigger>
    </Tabs.List>
      <Tabs.Content value="skills">
        <SkillList c={c} />
      </Tabs.Content>
      <Tabs.Content value="mcp">
        <McpList c={c} />
      </Tabs.Content>
      <Tabs.Content value="agents">
        <AgentList c={c} onConfigureAgent={onConfigureAgent} />
      </Tabs.Content>
      <Tabs.Content value="discover">
        <div className="discover-columns">
          <DiscoverGrid title="热门 Skill" icon={<Sparkles size={14} />} items={c.recommendedSkills} c={c} empty="暂无推荐" />
          <DiscoverGrid title="热门 MCP" icon={<PlugZap size={14} />} items={c.recommendedMcps} c={c} empty="暂无推荐" />
        </div>
      </Tabs.Content>
      <Tabs.Content value="config">
        <section className="extension-section">
          <div className="extension-section-heading"><h4>配置文件</h4></div>
          <p className="provider-panel-note">技能存放在 <code>~/.maestro/skills/</code>（全局）与 <code>&lt;workspace&gt;/.maestro/skills/</code>（项目级，覆盖同名全局技能）。MCP 配置位于 <code>~/.maestro/mcp.json</code>；斜杠命令位于 <code>~/.maestro/commands/</code> 与 <code>&lt;workspace&gt;/.maestro/commands/</code>。</p>
          <pre className="mcp-config-preview" aria-label="mcp.json 内容">{c.mcpConfigText || '{ "mcpServers": {} }'}</pre>
        </section>
      </Tabs.Content>
    </Tabs.Root>
  </section>;
}
