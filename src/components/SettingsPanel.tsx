import { useRef, useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import * as Slider from '@radix-ui/react-slider';
import { BrainCircuit, Globe2, Keyboard, Palette, Pencil, Plus, RotateCcw, Search, Settings2, Trash2, X } from 'lucide-react';
import LlmSettingsForm from './LlmSettingsForm';
import ConfirmDialog from './ConfirmDialog';
import { Button, Choice } from './ui';
import { bindShortcut, defaultShortcuts, formatShortcut, labels, shortcutChord, shortcutLabels, type ShortcutAction, type UiPreferences } from '../uiPreferences';
import { getProvider, readCustomProviders, removeCustomProvider, saveCustomProvider } from '../customProviders';
import { providers, type ProviderId } from '../modelCapabilities';
import { hasProviderCredential, readLlmSettings, selectLlmProvider } from '../settings';

type Section = 'general' | 'appearance' | 'models' | 'shortcuts';

export default function SettingsPanel({ open, onOpenChange, preview, preferences, onPreferences, onModelsChanged }: {
  open: boolean; onOpenChange: (open: boolean) => void; preview: boolean;
  preferences: UiPreferences; onPreferences: (value: UiPreferences) => void; onModelsChanged: () => void;
}) {
  const [section, setSection] = useState<Section>('general');
  const [customs, setCustoms] = useState(readCustomProviders);
  const [customDraft, setCustomDraft] = useState<{ id?: string; name: string; baseUrl: string } | null>(null);
  const [modelProvider, setModelProvider] = useState<ProviderId | (string & {})>(() => preview ? preferences.provider : readLlmSettings().provider ?? 'openai');
  const [providerError, setProviderError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [capture, setCapture] = useState<ShortcutAction | null>(null);
  const [bindingError, setBindingError] = useState<string | null>(null);
  const [credentialsTick, setCredentialsTick] = useState(0);
  const [deleteProvider, setDeleteProvider] = useState<{ id: string; name: string } | null>(null);
  const captureArea = useRef<HTMLDivElement>(null);
  const text = labels(preferences.language);
  const sections = [{ id: 'general', icon: Globe2 }, { id: 'appearance', icon: Palette }, { id: 'models', icon: BrainCircuit }, { id: 'shortcuts', icon: Keyboard }] as const;
  const update = (patch: Partial<UiPreferences>) => onPreferences({ ...preferences, ...patch });

  return <>
    <Dialog.Root open={open} onOpenChange={(value) => { setCapture(null); onOpenChange(value); }}>
    <Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="ide-settings-panel" onEscapeKeyDown={(event) => { if (capture) { event.preventDefault(); setCapture(null); } }} onKeyDown={(event) => {
      if (!capture || event.nativeEvent.isComposing) return;
      event.preventDefault(); event.stopPropagation();
      if (event.key === 'Escape') { setCapture(null); return; }
      const chord = shortcutChord(event);
      if (!chord) return;
      try { onPreferences(bindShortcut(preferences, capture, chord)); setCapture(null); setBindingError(null); }
      catch (error) { setBindingError(error instanceof Error ? error.message : '无效快捷键'); }
    }}>
      <div className="settings-rail"><Dialog.Title><Settings2 size={17} />{text.settings}</Dialog.Title>
        <nav aria-label="设置分类">{sections.map(({ id, icon: Icon }) => <button type="button" key={id} className="settings-nav-item" aria-current={section === id ? 'page' : undefined} onClick={() => { setSection(id); setCapture(null); }}><Icon size={15} />{text[id]}</button>)}</nav>
        <span className="settings-version">MAESTRO / 桌面版</span>
      </div>
      <div className="settings-content"><Dialog.Description className="settings-intro">{preview ? '前端预览 · 无真实配置读取或执行' : '连接配置与思考强度真实保存；上下文为模型能力参考，实际限制以服务端为准。'}</Dialog.Description>
        <h3>{text[section]}</h3>
        <div hidden={section !== 'general'}>
          <div className="preference-row"><div><strong>语言</strong><p>前端导航与设置标签</p></div><Choice label="界面语言" value={preferences.language} onChange={(value) => update({ language: value as UiPreferences['language'] })} options={[{ value: 'en', label: 'English' }, { value: 'zh-CN', label: '简体中文' }]} /></div>
          <div className="preference-row"><div><strong>启动视图</strong><p>下次打开时的工作区视图，不是 OS 自启动</p></div><Choice label="启动视图" value={preferences.startupView} onChange={(value) => update({ startupView: value as UiPreferences['startupView'] })} options={[{ value: 'agents', label: '会话' }, { value: 'activity', label: '活动' }, { value: 'extensions', label: '扩展' }]} /></div>
          <div className="settings-info-card">配置与预览偏好分别存储，不会改变真实模型权限或启动系统任务。</div>
        </div>
        <div hidden={section !== 'appearance'}>
          <div className="preference-row"><div><strong>主题</strong><p>深色、浅色或跟随系统外观</p></div><Choice label="主题" value={preferences.theme} onChange={(value) => update({ theme: value as UiPreferences['theme'] })} options={[{ value: 'dark', label: '深色' }, { value: 'light', label: '浅色' }, { value: 'system', label: '跟随系统' }]} /></div>
          <div className="preference-row"><div><strong>思考过程显示</strong><p>模型推理内容的展示方式</p></div><Choice label="思考过程显示" value={preferences.reasoningDisplay} onChange={(value) => update({ reasoningDisplay: value as UiPreferences['reasoningDisplay'] })} options={[{ value: 'collapsed', label: '收起' }, { value: 'summary', label: '摘要（前三行）' }, { value: 'hidden', label: '隐藏' }]} /></div>
          <div className="preference-row"><div><strong>界面缩放</strong><p>整体放大工作台 UI（默认 100%）</p></div><Choice label="界面缩放" value={preferences.fontScale} onChange={(value) => update({ fontScale: value as UiPreferences['fontScale'] })} options={[{ value: '100', label: '100%' }, { value: '125', label: '125%' }, { value: '150', label: '150%' }]} /></div>
          <div className="preference-row"><div><strong>编辑器字体</strong><p>系统等宽字体；不存在时回退到 Consolas</p></div><Choice label="编辑器字体" value={preferences.editorFont} onChange={(value) => update({ editorFont: value })} options={['Cascadia Code', 'Consolas', 'JetBrains Mono'].map((value) => ({ value, label: value }))} /></div>
          <div className="preference-row"><div><strong>字号</strong><p>{preferences.editorFontSize}px / Monaco</p></div><Slider.Root className="settings-slider font-size-slider" value={[preferences.editorFontSize]} min={10} max={20} step={1} onValueChange={([value]) => update({ editorFontSize: value })}><Slider.Track><Slider.Range /></Slider.Track><Slider.Thumb aria-label="编辑器字号" /></Slider.Root></div>
          <pre className="font-preview" style={{ fontFamily: `"${preferences.editorFont}", Consolas, monospace`, fontSize: preferences.editorFontSize }}>const session = await agent.plan();</pre>
        </div>
        <div hidden={section !== 'models'}>
          <div className="models-two-pane" data-tick={credentialsTick}>
            <div className="model-provider-list" role="group" aria-label="供应商">
              {(Object.keys(providers) as ProviderId[]).map((id) => <button type="button" key={id} className="model-provider-item" aria-label={getProvider(id).name} aria-current={modelProvider === id ? 'page' : undefined} onClick={() => {
                try {
                  if (preview) update({ provider: id, previewModel: getProvider(id).model }); else { selectLlmProvider(id); onModelsChanged(); }
                  setModelProvider(id); setProviderError(null);
                } catch (error) { setProviderError(error instanceof Error ? error.message : 'Cannot change provider'); }
              }}>
                <span className="provider-card-icon">{getProvider(id).name.slice(0, 1)}</span>{getProvider(id).name}
                <span className={`provider-status-dot ${preview || hasProviderCredential(id) ? 'configured' : ''}`} />
              </button>)}
              {customs.map((item) => <div role="button" tabIndex={0} key={item.id} className="model-provider-item" aria-label={item.name} aria-current={modelProvider === item.id ? 'page' : undefined} aria-disabled={preview || undefined}
                onKeyDown={(event) => { if (event.key === 'Enter') event.currentTarget.click(); }}
                onClick={() => {
                try {
                  if (preview) update({ provider: item.id as ProviderId, previewModel: item.baseUrl ? '' : item.id }); else { selectLlmProvider(item.id); onModelsChanged(); }
                  setModelProvider(item.id); setProviderError(null);
                } catch (error) { setProviderError(error instanceof Error ? error.message : 'Cannot change provider'); }
              }}>
                <span className="provider-card-icon">{item.name.slice(0, 1)}</span>{item.name}
                <span className={`provider-status-dot ${preview || hasProviderCredential(item.id) ? 'configured' : ''}`} />
                <span className="provider-item-actions" onClick={(event) => event.stopPropagation()}>
                  <button type="button" className="icon-button" aria-label={`编辑供应商 ${item.name}`} disabled={preview} onClick={() => setCustomDraft({ id: item.id, name: item.name ?? '', baseUrl: item.baseUrl ?? '' })}><Pencil size={11} /></button>
                  <button type="button" className="icon-button" aria-label={`删除供应商 ${item.name}`} disabled={preview} onClick={() => setDeleteProvider({ id: item.id, name: item.name ?? item.id })}><Trash2 size={11} /></button>
                </span>
              </div>)}
              <button type="button" className="model-provider-item model-provider-add" aria-label="添加自定义供应商" disabled={preview}
                onClick={() => setCustomDraft({ name: '', baseUrl: '' })}><Plus size={13} />自定义供应商</button>
            </div>
            <div className="model-provider-detail">
              <div className="provider-panel-heading"><span className="provider-card-icon">{getProvider(modelProvider).name.slice(0, 1)}</span>
                <strong>{getProvider(modelProvider).name}</strong>
                <span className={`provider-badge ${preview || hasProviderCredential(modelProvider) ? 'configured' : ''}`}>{preview ? '预览' : hasProviderCredential(modelProvider) ? '已配置' : '未配置'}</span>
              </div>
              <p className="provider-panel-note">独立 Provider 配置，凭据不会在 Provider 之间复制。</p>
              {providerError && <p role="alert" className="settings-form-error">{providerError}</p>}
              <LlmSettingsForm provider={modelProvider} preview={preview} onSaved={() => { setCredentialsTick((tick) => tick + 1); onModelsChanged(); onOpenChange(false); }} onChanged={() => { setCredentialsTick((tick) => tick + 1); onModelsChanged(); }} />
            </div>
          </div>
        </div>
        <div hidden={section !== 'shortcuts'}>
          <label className="shortcut-search"><Search size={14} /><input aria-label="搜索快捷键" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索快捷键…" /></label>
          <div className="shortcut-row"><div><strong>发送消息</strong><small>保留键 / 输入法安全</small></div><kbd>Ctrl / Cmd + Enter</kbd></div>
          {(Object.keys(shortcutLabels) as ShortcutAction[]).filter((action) => `${shortcutLabels[action]} ${formatShortcut(preferences.shortcuts[action])}`.toLowerCase().includes(query.toLowerCase())).map((action) => <div key={action} className="shortcut-row">
            <span>{shortcutLabels[action]}</span><button type="button" className="shortcut-binding" aria-label={`重新绑定 ${shortcutLabels[action]}`} onClick={() => { setCapture(action); setBindingError(null); requestAnimationFrame(() => captureArea.current?.focus()); }}><Keyboard size={12} /><kbd>{formatShortcut(preferences.shortcuts[action])}</kbd></button>
            <button type="button" className="icon-button" aria-label={`重置 ${shortcutLabels[action]}`} onClick={() => { try { onPreferences(bindShortcut(preferences, action, defaultShortcuts[action])); setBindingError(null); } catch { setBindingError('默认按键已被其他动作使用。'); } }}><RotateCcw size={12} /></button>
          </div>)}
          <div ref={captureArea} tabIndex={-1} className="shortcut-capture" hidden={!capture}>按下新的组合键。Escape 取消；仅作用于应用内 UI。</div>
          {bindingError && <p role="alert" className="settings-form-error">{bindingError}</p>}
        </div>
      </div>
      <Dialog.Close asChild><button type="button" className="icon-button dialog-close" aria-label="关闭设置"><X size={16} /></button></Dialog.Close>
      {customDraft && <div className="custom-provider-dialog" role="dialog" aria-label="自定义供应商"
        onClick={(event) => { if (event.target === event.currentTarget) setCustomDraft(null); }}>
        <div className="custom-provider-card">
          <h4>{customDraft.id ? '编辑供应商' : '添加自定义供应商'}</h4>
          <div className="modal-field">
            <label htmlFor="custom-provider-name">名称</label>
            <input id="custom-provider-name" autoComplete="off" autoFocus value={customDraft.name} maxLength={40}
              onChange={(event) => setCustomDraft({ ...customDraft, name: event.target.value })} placeholder="如 OpenCode go" />
          </div>
          <div className="modal-field">
            <label htmlFor="custom-provider-url">Base URL（OpenAI-compatible）</label>
            <input id="custom-provider-url" autoComplete="off" spellCheck={false} value={customDraft.baseUrl}
              onChange={(event) => setCustomDraft({ ...customDraft, baseUrl: event.target.value })} placeholder="https://gateway.example/v1" />
          </div>
          <div className="dialog-actions">
            <Button variant="ghost" onClick={() => setCustomDraft(null)}>取消</Button>
            <Button variant="primary" disabled={!(customDraft.name ?? '').trim() || !(customDraft.baseUrl ?? '').trim()} onClick={() => {
              try {
                const row = saveCustomProvider(customDraft);
                setCustoms(readCustomProviders());
                setModelProvider(row.id);
                if (!preview) { selectLlmProvider(row.id); onModelsChanged(); }
                setCustomDraft(null);
              } catch (error) { setProviderError(error instanceof Error ? error.message : '无法保存供应商'); setCustomDraft(null); }
            }}>保存</Button>
          </div>
        </div>
      </div>}
    </Dialog.Content></Dialog.Portal>
  </Dialog.Root>
  <ConfirmDialog open={deleteProvider !== null} onOpenChange={(open) => { if (!open) setDeleteProvider(null); }}
    title="删除自定义供应商"
    description={deleteProvider ? `删除自定义供应商「${deleteProvider.name}」及其本地配置？此操作不可撤销。` : ''}
    onConfirm={() => {
      if (!deleteProvider) return;
      removeCustomProvider(deleteProvider.id);
      for (const key of [`maestro.llm.models.${deleteProvider.id}`, `maestro.llm.models.removed.${deleteProvider.id}`, `maestro.llm.fetched.${deleteProvider.id}`]) localStorage.removeItem(key);
      setCustoms(readCustomProviders());
      if (modelProvider === deleteProvider.id) { setModelProvider('openai'); if (!preview) selectLlmProvider('openai'); }
    }} />
  </>;
}
