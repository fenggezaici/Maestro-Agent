import { useRef, useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import * as Slider from '@radix-ui/react-slider';
import { BrainCircuit, Globe2, Keyboard, Palette, RotateCcw, Search, Settings2, X } from 'lucide-react';
import LlmSettingsForm from './LlmSettingsForm';
import { Choice } from './ui';
import { bindShortcut, defaultShortcuts, formatShortcut, labels, shortcutChord, shortcutLabels, type ShortcutAction, type UiPreferences } from '../uiPreferences';

type Section = 'general' | 'appearance' | 'models' | 'shortcuts';

export default function SettingsPanel({ open, onOpenChange, preview, preferences, onPreferences, onModelsChanged }: {
  open: boolean; onOpenChange: (open: boolean) => void; preview: boolean;
  preferences: UiPreferences; onPreferences: (value: UiPreferences) => void; onModelsChanged: () => void;
}) {
  const [section, setSection] = useState<Section>('general');
  const [query, setQuery] = useState('');
  const [capture, setCapture] = useState<ShortcutAction | null>(null);
  const [bindingError, setBindingError] = useState<string | null>(null);
  const captureArea = useRef<HTMLDivElement>(null);
  const text = labels(preferences.language);
  const sections = [{ id: 'general', icon: Globe2 }, { id: 'appearance', icon: Palette }, { id: 'models', icon: BrainCircuit }, { id: 'shortcuts', icon: Keyboard }] as const;
  const update = (patch: Partial<UiPreferences>) => onPreferences({ ...preferences, ...patch });

  return <Dialog.Root open={open} onOpenChange={(value) => { setCapture(null); onOpenChange(value); }}>
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
        <span className="settings-version">MAESTRO / DESKTOP</span>
      </div>
      <div className="settings-content"><Dialog.Description className="settings-intro">{preview ? 'UI preview · 无真实配置读取或执行' : '连接配置真实保存，其余新增能力为 UI Mock'}</Dialog.Description>
        <h3>{text[section]}</h3>
        <div hidden={section !== 'general'}>
          <div className="preference-row"><div><strong>Language</strong><p>前端导航与设置标签</p></div><Choice label="界面语言" value={preferences.language} onChange={(value) => update({ language: value as UiPreferences['language'] })} options={[{ value: 'en', label: 'English' }, { value: 'zh-CN', label: '简体中文' }]} /></div>
          <div className="preference-row"><div><strong>On startup</strong><p>下次打开时的工作区视图，不是 OS 自启动</p></div><Choice label="启动视图" value={preferences.startupView} onChange={(value) => update({ startupView: value as UiPreferences['startupView'] })} options={[{ value: 'agents', label: 'Agents' }, { value: 'activity', label: 'Activity' }, { value: 'extensions', label: 'Extensions' }]} /></div>
          <div className="settings-info-card">配置与预览偏好分别存储，不会改变真实模型权限或启动系统任务。</div>
        </div>
        <div hidden={section !== 'appearance'}>
          <div className="preference-row"><div><strong>Theme</strong><p>主题选项 Mock，工作台保持 Dark</p></div><Choice label="主题" value={preferences.theme} onChange={(value) => update({ theme: value as UiPreferences['theme'] })} options={[{ value: 'dark', label: 'Dark' }, { value: 'light', label: 'Light' }, { value: 'system', label: 'System' }]} /></div>
          <div className="preference-row"><div><strong>Editor font</strong><p>系统等宽字体；不存在时回退到 Consolas</p></div><Choice label="编辑器字体" value={preferences.editorFont} onChange={(value) => update({ editorFont: value })} options={['Cascadia Code', 'Consolas', 'JetBrains Mono'].map((value) => ({ value, label: value }))} /></div>
          <div className="preference-row"><div><strong>Font size</strong><p>{preferences.editorFontSize}px / Monaco</p></div><Slider.Root className="settings-slider font-size-slider" value={[preferences.editorFontSize]} min={10} max={20} step={1} onValueChange={([value]) => update({ editorFontSize: value })}><Slider.Track><Slider.Range /></Slider.Track><Slider.Thumb aria-label="编辑器字号" /></Slider.Root></div>
          <pre className="font-preview" style={{ fontFamily: `"${preferences.editorFont}", Consolas, monospace`, fontSize: preferences.editorFontSize }}>const session = await agent.plan();</pre>
        </div>
        <div hidden={section !== 'models'}>
          <div className="preference-row"><div><strong>Temperature</strong><p>Mock UI parameter / not sent to model</p></div><div className="temperature-setting"><span>{preferences.temperature.toFixed(1)}</span><Slider.Root className="settings-slider" value={[preferences.temperature]} min={0} max={2} step={0.1} onValueChange={([value]) => update({ temperature: value })}><Slider.Track><Slider.Range /></Slider.Track><Slider.Thumb aria-label="Mock Temperature" /></Slider.Root></div></div>
          <div className="preference-row"><div><strong>Provider</strong><p>Provider 切换仅 UI，实际传输仍为 OpenAI-compatible</p></div><Choice label="Provider（Mock）" value={preferences.provider} onChange={(value) => update({ provider: value as UiPreferences['provider'] })} options={[{ value: 'openai', label: 'OpenAI' }, { value: 'anthropic', label: 'Anthropic' }, { value: 'ollama', label: 'Ollama' }]} /></div>
          {preferences.provider !== 'openai' && <div className="settings-info-card">{preferences.provider === 'anthropic' ? 'Anthropic 原生协议尚未接入。' : 'Ollama 原生协议尚未接入，可手动配置兼容端点。'} 此选择不覆盖 URL、Key 或 Model。</div>}
          <LlmSettingsForm preview={preview} onSaved={() => { onModelsChanged(); onOpenChange(false); }} onChanged={onModelsChanged} />
        </div>
        <div hidden={section !== 'shortcuts'}>
          <label className="shortcut-search"><Search size={14} /><input aria-label="搜索快捷键" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search shortcuts…" /></label>
          <div className="shortcut-row"><div><strong>Send message</strong><small>Reserved / IME-safe</small></div><kbd>Ctrl / Cmd + Enter</kbd></div>
          {(Object.keys(shortcutLabels) as ShortcutAction[]).filter((action) => `${shortcutLabels[action]} ${formatShortcut(preferences.shortcuts[action])}`.toLowerCase().includes(query.toLowerCase())).map((action) => <div key={action} className="shortcut-row">
            <span>{shortcutLabels[action]}</span><button type="button" className="shortcut-binding" aria-label={`重新绑定 ${shortcutLabels[action]}`} onClick={() => { setCapture(action); setBindingError(null); requestAnimationFrame(() => captureArea.current?.focus()); }}><Keyboard size={12} /><kbd>{formatShortcut(preferences.shortcuts[action])}</kbd></button>
            <button type="button" className="icon-button" aria-label={`重置 ${shortcutLabels[action]}`} onClick={() => { try { onPreferences(bindShortcut(preferences, action, defaultShortcuts[action])); setBindingError(null); } catch { setBindingError('默认按键已被其他动作使用。'); } }}><RotateCcw size={12} /></button>
          </div>)}
          <div ref={captureArea} tabIndex={-1} className="shortcut-capture" hidden={!capture}>按下新的组合键。Escape 取消；仅作用于应用内 UI。</div>
          {bindingError && <p role="alert" className="settings-form-error">{bindingError}</p>}
        </div>
      </div>
      <Dialog.Close asChild><button type="button" className="icon-button dialog-close" aria-label="关闭设置"><X size={16} /></button></Dialog.Close>
    </Dialog.Content></Dialog.Portal>
  </Dialog.Root>;
}
