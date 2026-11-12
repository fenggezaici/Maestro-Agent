import { useState, type FormEvent } from 'react';
import { useT } from '../i18n';
import * as Dialog from '@radix-ui/react-dialog';
import * as Tabs from '@radix-ui/react-tabs';
import * as Switch from '@radix-ui/react-switch';
import { ExternalLink, FolderInput, FolderOpen, GitBranch, PlugZap, Plus, Puzzle, Sparkles, Trash2, X } from 'lucide-react';
import type { ExtensionsController } from '../hooks/useExtensions';
import type { McpServerView, RecommendedItem } from '../api/extensions';
import { pickDirectory, pickFile } from '../api/dialog';
import { IconButton } from './ui';

function FormDialog({ open, onOpenChange, title, hint, submitLabel, fields, canBrowse = false, onSubmit }: {
  open: boolean; onOpenChange: (open: boolean) => void; title: string; hint?: string;
  submitLabel: string;
  canBrowse?: boolean;
  fields: { id: string; label: string; placeholder?: string; required?: boolean; textarea?: boolean; browse?: 'directory' | 'file' }[];
  onSubmit: (values: Record<string, string>) => Promise<string | null>;
}) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    setPending(true);
    const message = await onSubmit(values);
    setPending(false);
    if (message) { setError(message); return; }
    setValues({}); setError(null); onOpenChange(false);
  }
  return <Dialog.Root open={open} onOpenChange={(value) => { setError(null); onOpenChange(value); }}>
    <Dialog.Portal><Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="project-dialog" aria-describedby={undefined}>
        <Dialog.Title asChild><h3>{title}</h3></Dialog.Title>
        {hint && <p className="dialog-hint">{hint}</p>}
        <form onSubmit={(event) => void submit(event)}>
          {fields.map((field) => <div key={field.id}>
            <label htmlFor={`ext-${field.id}`}>{field.label}{field.required !== false && ' *'}</label>
            {field.textarea
              ? <textarea id={`ext-${field.id}`} rows={3} value={values[field.id] ?? ''} onChange={(e) => setValues((v) => ({ ...v, [field.id]: e.target.value }))} placeholder={field.placeholder} />
              : <div className="field-with-browse">
                <input id={`ext-${field.id}`} autoComplete="off" spellCheck={false} value={values[field.id] ?? ''} onChange={(e) => setValues((v) => ({ ...v, [field.id]: e.target.value }))} placeholder={field.placeholder} />
                {field.browse && <IconButton label={field.browse === 'directory' ? '浏览目录' : '浏览文件'} disabled={!canBrowse}
                  onClick={async () => {
                    const picked = field.browse === 'directory' ? await pickDirectory() : await pickFile(['json']);
                    if (picked) setValues((v) => ({ ...v, [field.id]: picked }));
                  }}><FolderOpen size={14} /></IconButton>}
              </div>}
          </div>)}
          {error && <p role="alert" className="settings-form-error">{error}</p>}
          <div className="dialog-actions">
            <Dialog.Close asChild><button type="button" className="button-ghost">取消</button></Dialog.Close>
            <button type="submit" className="button-approve" disabled={pending || fields.some((f) => f.required !== false && !(values[f.id] ?? '').trim())}>{submitLabel}</button>
          </div>
        </form>
        <Dialog.Close asChild><button type="button" className="icon-button dialog-close" aria-label="关闭"><X size={15} /></button></Dialog.Close>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

function SkillList({ c }: { c: ExtensionsController }) {
  const [dialog, setDialog] = useState<'create' | 'local' | 'github' | null>(null);
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
    {c.importSummary && <p className="import-summary">{c.importSummary}</p>}
    {c.skills.map((skill) => <article className="extension-card" key={skill.name}>
      <span className="extension-icon"><Sparkles size={18} /></span>
      <div className="extension-description"><strong>{skill.name}</strong><p>{skill.description || '（无描述）'}</p>
        <span className="extension-state">{skill.source === 'project' ? '项目级' : '全局'} · {skill.path}</span></div>
    </article>)}
    <FormDialog open={dialog === 'create'} onOpenChange={(o) => !o && setDialog(null)} title="新建技能"
      hint="在 ~/.maestro/skills/ 下生成标准 SKILL.md 模板。" submitLabel="创建"
      fields={[{ id: 'name', label: '名称（kebab-case）', placeholder: 'my-skill' }, { id: 'description', label: '描述', placeholder: '这个技能做什么？', textarea: true }]}
      onSubmit={(v) => c.createSkill(v.name, v.description)} />
    <FormDialog open={dialog === 'local'} onOpenChange={(o) => !o && setDialog(null)} title="从本地导入技能"
      hint="选择 SKILL.md 文件、单个技能目录，或含多个技能目录的父目录（批量导入）。" submitLabel="导入" canBrowse={!!c.real}
      fields={[{ id: 'path', label: '本地路径', placeholder: 'E:\\skills\\my-skill', browse: 'directory' }]}
      onSubmit={(v) => c.importSkill(v.path)} />
    <FormDialog open={dialog === 'github'} onOpenChange={(o) => !o && setDialog(null)} title="从 GitHub 导入技能"
      hint="克隆仓库并递归导入其中全部 SKILL.md。" submitLabel="导入"
      fields={[{ id: 'repo', label: '仓库地址', placeholder: 'https://github.com/owner/repo' }]}
      onSubmit={(v) => c.importSkillGithub(v.repo)} />
  </section>;
}

function McpList({ c }: { c: ExtensionsController }) {
  const [dialog, setDialog] = useState<'add' | 'import' | null>(null);
  return <section aria-label="MCP 服务列表" className="extension-section">
    <div className="extension-section-heading">
      <h4><PlugZap size={14} />MCP 服务</h4>
      <div className="extension-actions">
        <button type="button" className="button-ghost" onClick={() => setDialog('add')}><Plus size={13} />添加配置</button>
        <button type="button" className="button-ghost" onClick={() => setDialog('import')}><FolderInput size={13} />导入 JSON</button>
      </div>
    </div>
    {c.servers.length === 0 && <p className="sidebar-empty">{c.loading ? '加载中…' : '暂无 MCP 服务。添加配置或从发现页安装。'}</p>}
    {c.servers.map((server) => <McpRow key={server.name} c={c} server={server} />)}
    <FormDialog open={dialog === 'add'} onOpenChange={(o) => !o && setDialog(null)} title="添加 MCP 服务"
      hint="写入 ~/.maestro/mcp.json；开启后拉起子进程并把工具注入对话。" submitLabel="添加"
      fields={[{ id: 'name', label: '名称', placeholder: 'filesystem' }, { id: 'command', label: '启动命令', placeholder: 'npx' }, { id: 'args', label: '参数（空格分隔，可选）', placeholder: '-y @modelcontextprotocol/server-filesystem', required: false }]}
      onSubmit={(v) => c.addMcpServer(v.name, v.command, (v.args ?? '').split(' ').filter(Boolean))} />
    <FormDialog open={dialog === 'import'} onOpenChange={(o) => !o && setDialog(null)} title="导入 MCP JSON"
      hint="读取一个含 mcpServers 字段的 JSON 文件并合并到 ~/.maestro/mcp.json。" submitLabel="导入" canBrowse={!!c.real}
      fields={[{ id: 'path', label: 'JSON 文件路径', placeholder: 'E:\\config\\mcp.json', browse: 'file' }]}
      onSubmit={(v) => c.importMcpJson(v.path)} />
  </section>;
}

function McpRow({ c, server }: { c: ExtensionsController; server: McpServerView }) {
  const busy = c.pending === `mcp-${server.name}`;
  return <article className="extension-card" key={server.name}>
    <span className={`extension-status-dot ${server.running ? 'running' : server.error ? 'error' : ''}`} aria-hidden="true" />
    <div className="extension-description">
      <strong>{server.name}</strong>
      <p><code>{server.command} {server.args.join(' ')}</code></p>
      <span className="extension-state">
        {busy ? '处理中…' : server.running ? `运行中 · 已加载 ${server.toolCount} 个工具` : server.error ? server.error : server.disabled ? '已禁用' : '已停止'}
      </span>
    </div>
    <span className="extension-state" title="只有显式信任的服务才允许其 readOnlyHint 生效">信任只读</span>
    <Switch.Root aria-label={`信任只读声明 ${server.name}`} className="settings-switch" checked={server.trust} disabled={busy}
      onCheckedChange={(trust) => void c.setMcpTrust(server, trust)}><Switch.Thumb /></Switch.Root>
    <Switch.Root aria-label={`启用 ${server.name}`} className="settings-switch" checked={server.running} disabled={busy}
      onCheckedChange={(enable) => void c.toggleMcpServer(server, enable)}><Switch.Thumb /></Switch.Root>
    <button type="button" className="icon-button" aria-label={`删除 ${server.name}`} disabled={busy} onClick={() => void c.removeMcpServer(server.name)}><Trash2 size={13} /></button>
  </article>;
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

export default function ExtensionsView({ controller: c }: { controller: ExtensionsController }) {
  const t = useT();
  return <section aria-label="扩展工作区" className="extensions-view">
    <header className="extensions-heading"><span className="welcome-icon"><Puzzle size={26} strokeWidth={1.2} /></span><div><p className="eyebrow">{t.workspaceCapabilities}</p><h3>{t.extensions}</h3><p>Skills 与 MCP 的完整生命周期管理</p></div></header>
    <Tabs.Root defaultValue="skills" className="extensions-tabs"><Tabs.List aria-label="扩展分类">
      <Tabs.Trigger value="skills">技能</Tabs.Trigger>
      <Tabs.Trigger value="mcp">MCP 服务</Tabs.Trigger>
      <Tabs.Trigger value="discover">发现</Tabs.Trigger>
      <Tabs.Trigger value="config">配置</Tabs.Trigger>
    </Tabs.List>
      <Tabs.Content value="skills">
        <SkillList c={c} />
      </Tabs.Content>
      <Tabs.Content value="mcp">
        <McpList c={c} />
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
          <p className="provider-panel-note">技能存放在 <code>~/.maestro/skills/</code>（全局）与 <code>&lt;workspace&gt;/.maestro/skills/</code>（项目级，覆盖同名全局技能）。MCP 配置位于 <code>~/.maestro/mcp.json</code>。</p>
          <pre className="mcp-config-preview" aria-label="mcp.json 内容">{c.mcpConfigText || '{ "mcpServers": {} }'}</pre>
        </section>
      </Tabs.Content>
    </Tabs.Root>
  </section>;
}
