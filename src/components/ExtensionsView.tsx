import * as Tabs from '@radix-ui/react-tabs';
import * as Switch from '@radix-ui/react-switch';
import { Blocks, PlugZap, Puzzle, Sparkles, Trash2 } from 'lucide-react';
import type { MockExtension } from './workbenchMock';

export default function ExtensionsView({ items, onAction }: { items: MockExtension[]; onAction: (id: string, action: 'toggle' | 'install' | 'remove') => void }) {
  return <section aria-label="Extensions 工作区" className="extensions-view">
    <header className="extensions-heading"><span className="welcome-icon"><Puzzle size={26} strokeWidth={1.2} /></span><div><p className="eyebrow">WORKSPACE CAPABILITIES</p><h3>Extensions</h3><p>MCP, Skills and Plugins / UI demonstration</p></div><span className="mock-badge">Mock</span></header>
    <p className="extensions-disclaimer">所有安装状态和开关仅为本地 Mock，不下载插件、不启动 MCP、不执行 Skills。</p>
    <Tabs.Root defaultValue="mcp" className="extensions-tabs"><Tabs.List aria-label="扩展分类"><Tabs.Trigger value="mcp"><PlugZap size={14} />MCP</Tabs.Trigger><Tabs.Trigger value="skills"><Sparkles size={14} />Skills</Tabs.Trigger><Tabs.Trigger value="plugins"><Blocks size={14} />Plugins</Tabs.Trigger></Tabs.List>
      {(['mcp', 'skills', 'plugins'] as const).map((kind) => <Tabs.Content key={kind} value={kind}>
        {items.filter((item) => item.kind === kind).map((item) => <article className="extension-card" key={item.id}>
          <span className="extension-icon">{kind === 'mcp' ? <PlugZap size={18} /> : kind === 'skills' ? <Sparkles size={18} /> : <Blocks size={18} />}</span>
          <div className="extension-description"><strong>{item.name}</strong><p>{item.description}</p><span className="extension-state">{item.status === 'installed' ? item.enabled ? 'Enabled / Mock' : 'Disabled / Mock' : item.status === 'available' ? 'Available / Mock' : 'Unavailable / Mock'}</span></div>
          {item.status === 'available' ? <button type="button" className="button-ghost" aria-label={`模拟安装 ${item.name}`} onClick={() => onAction(item.id, 'install')}>Install</button>
            : <><Switch.Root aria-label={`启用 ${item.name}`} className="settings-switch" checked={item.enabled} disabled={item.status !== 'installed'} onCheckedChange={() => onAction(item.id, 'toggle')}><Switch.Thumb /></Switch.Root>
              {item.status === 'installed' && <button type="button" className="icon-button" aria-label={`模拟移除 ${item.name}`} onClick={() => onAction(item.id, 'remove')}><Trash2 size={13} /></button>}</>}
        </article>)}
      </Tabs.Content>)}
    </Tabs.Root>
  </section>;
}
