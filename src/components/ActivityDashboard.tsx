import { ArrowLeft, CalendarRange, ChevronDown, Clock3, FileCode2, Flame, PieChart as PieIcon, Sparkles, TrendingUp, Zap } from 'lucide-react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { useMemo, useState } from 'react';
import { useT } from '../i18n';
import { CartesianGrid, Cell, Line, LineChart, Pie, PieChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from 'recharts';
import ActivityGrid from './ActivityGrid';
import { activityStreaks, type DashboardData } from './activityMock';

export type ActivityScope = { id: 'all' | number; name: string };

const MODEL_COLORS = ['#4e9af1', '#65a374', '#c98bd9', '#e0a35c', '#e06c6c', '#6cc7c7', '#b8a2d1'];

// 与 ZCode 用量统计一致的紧凑计数：中文亿/万，英文 M/k。
export function formatTokens(value: number, zh: boolean): string {
  if (zh) {
    if (value >= 100000000) return `${(value / 100000000).toFixed(1)} 亿`;
    if (value >= 10000) return `${(value / 10000).toFixed(1)} 万`;
    return String(value);
  }
  if (value >= 1000000) return `${(value / 1000000).toFixed(1)}M`;
  if (value >= 1000) return `${(value / 1000).toFixed(1)}k`;
  return String(value);
}

export default function ActivityDashboard({ data, projectName, scopes, scope, onScopeChange, onBack, loading = false, error = null }: {
  data: DashboardData; projectName: string; scopes?: ActivityScope[]; scope?: 'all' | number;
  onScopeChange?: (scope: 'all' | number) => void; onBack: () => void; loading?: boolean; error?: string | null;
}) {
  const t = useT();
  const scopeName = scope === 'all' ? t.actScopeAll : projectName;
  const zh = t.locale === 'zh-CN';
  const live = data.source === 'live';
  const [range, setRange] = useState<7 | 30>(30);
  const streaks = useMemo(() => activityStreaks(data.days), [data.days]);
  const modelNames = useMemo(() => data.models.map((entry) => entry.model), [data.models]);
  const trendRows = useMemo(() => {
    const window = data.days.slice(-range);
    const trendByDate = new Map(data.modelTrend.map((entry) => [`${entry.date}|${entry.model}`, entry.tokens]));
    return window.map((day) => {
      const row: Record<string, number | string> = { shortDate: day.shortDate, total: day.tokens };
      for (const model of modelNames) row[model] = trendByDate.get(`${day.date}|${model}`) ?? 0;
      return row;
    });
  }, [data.days, data.modelTrend, modelNames, range]);
  const trendTotal = trendRows.reduce((total, day) => total + (day.total as number), 0);
  const modelTotal = data.models.reduce((total, entry) => total + entry.tokens, 0);
  const metrics = [
    { label: t.actTotalLines, value: data.totalLines.toLocaleString(zh ? 'zh-CN' : 'en-US'), detail: t.actLinesChanged, icon: FileCode2 },
    { label: t.actTotalTokens, value: formatTokens(data.totalTokens, zh), detail: t.actModelTokens, icon: Sparkles },
    { label: t.actPeakTokens, value: formatTokens(data.peakTokens, zh), detail: t.actPeakDetail, icon: Zap },
    { label: t.actLongestRun, value: t.actDuration(data.longestRunMs), detail: t.actRunDetail, icon: Clock3 },
    { label: t.actCurrentStreak, value: `${streaks.current} ${t.actDayUnit}`, detail: t.actStreakDetail, icon: Flame },
    { label: t.actLongestStreak, value: `${streaks.longest} ${t.actDayUnit}`, detail: t.actStreakDetail, icon: CalendarRange },
  ];
  return <section aria-label="Activity Dashboard" className="dashboard-scroll">
    <div className="dashboard-inner">
      <button type="button" className="dashboard-back" onClick={onBack}><ArrowLeft size={13} />{t.backToChat}</button>
      <div className="dashboard-title"><div><p className="eyebrow">{t.workspaceInsights}</p><h3>{t.activityDashboard}</h3><p>
        {scopes && onScopeChange ? <DropdownMenu.Root>
          <DropdownMenu.Trigger className="activity-scope-trigger" aria-label={t.actScopeAria}>{scopeName}<ChevronDown size={12} /></DropdownMenu.Trigger>
          <DropdownMenu.Portal><DropdownMenu.Content sideOffset={4} align="start" className="dropdown-content">
            {scopes.map((item) => <DropdownMenu.Item key={String(item.id)} className="dropdown-item" onSelect={() => onScopeChange(item.id)}>{item.id === 'all' ? t.actScopeAll : item.name}</DropdownMenu.Item>)}
          </DropdownMenu.Content></DropdownMenu.Portal>
        </DropdownMenu.Root> : scopeName}
        {` / ${t.actPastYear}`}</p></div>{!live && <span className="mock-badge">Mock data</span>}</div>
      {loading && <p className="dashboard-disclaimer">{t.actLoading}</p>}
      {error && <p role="alert" className="dashboard-disclaimer">{t.actLoadFailed}：{error}</p>}
      {!live && <p className="dashboard-disclaimer">{t.actMockNotice}</p>}
      {live && data.totalLines === 0 && <p className="dashboard-disclaimer">{t.actRunCommandNote}</p>}
      <div className="dashboard-metrics" aria-label="核心指标">
        {metrics.map(({ label, value, detail, icon: Icon }) => <article key={label} aria-label={live ? label : `${label}（Mock）`} className="metric-card">
          <div className="metric-heading"><Icon size={14} strokeWidth={1.5} />{!live && <span className="mock-badge">Mock</span>}</div>
          <p>{label}</p><strong>{value}</strong><small>{detail}</small>
        </article>)}
      </div>
      <ActivityGrid days={data.days} live={live} />
      <section aria-label={t.actTrendTitle} className="dashboard-card trend-card">
        <header className="dashboard-section-heading"><div><h4><TrendingUp size={15} />{t.actTrendTitle}</h4><p>{`${range === 7 ? t.actLast7 : t.actLast30} / ${trendTotal.toLocaleString(zh ? 'zh-CN' : 'en-US')} tokens`}</p></div>
          <div className="segmented-control" role="group" aria-label="时间范围">
            {([7, 30] as const).map((days) => <button key={days} type="button" className={range === days ? 'active' : ''} onClick={() => setRange(days)}>{days === 7 ? t.actLast7 : t.actLast30}</button>)}
          </div>
        </header>
        <div className="trend-chart" role="img" aria-label={live ? 'Token 消耗折线图' : 'Token 消耗折线图（Mock）'}>
          <ResponsiveContainer width="100%" height="100%" minWidth={0} initialDimension={{ width: 520, height: 205 }}>
            <LineChart data={trendRows} margin={{ top: 15, right: 8, bottom: 0, left: -14 }}>
              <CartesianGrid stroke="#ffffff08" strokeDasharray="3 5" vertical={false} />
              <XAxis dataKey="shortDate" axisLine={false} tickLine={false} minTickGap={42} tick={{ fill: '#7d7d7d', fontSize: 9 }} tickMargin={12} />
              <YAxis axisLine={false} tickLine={false} width={44} tick={{ fill: '#7d7d7d', fontSize: 9 }} tickFormatter={(value: number) => formatTokens(value, zh)} />
              <ChartTooltip cursor={{ stroke: '#ffffff15' }} content={({ active, payload, label }) => active && payload?.length
                ? <div className="activity-tooltip"><strong>{label}{live ? '' : ' / Mock'}</strong>{payload.map((entry) => <p key={String(entry.dataKey)}><span className="tooltip-dot" style={{ background: String(entry.color) }} />{entry.name}：{Number(entry.value).toLocaleString(zh ? 'zh-CN' : 'en-US')}</p>)}</div> : null} />
              {modelNames.length > 0
                ? modelNames.map((model, index) => <Line key={model} type="monotone" dataKey={model} name={model} stroke={MODEL_COLORS[index % MODEL_COLORS.length]} strokeWidth={2} dot={false} isAnimationActive={false} />)
                : <Line type="monotone" dataKey="total" name="tokens" stroke="#aa97c3" strokeWidth={2} dot={false} activeDot={{ r: 4, fill: '#b8a2d1', stroke: '#242424', strokeWidth: 2 }} isAnimationActive={false} />}
            </LineChart>
          </ResponsiveContainer>
        </div>
        {modelNames.length > 0 && <div className="trend-legend">{modelNames.map((model, index) => <span key={model}><i style={{ background: MODEL_COLORS[index % MODEL_COLORS.length] }} />{model}</span>)}</div>}
        <p className="trend-caption">{live ? t.actTrendCaption(range) : `${range} ${zh ? '个每日样本 · 示例趋势，不连接用量或计费 API。' : 'daily samples · example trend, not connected to usage or billing APIs.'}`}</p>
      </section>
      {live && data.models.length > 0 && <section aria-label={t.actModelUsage} className="dashboard-card">
        <header className="dashboard-section-heading"><div><h4><PieIcon size={15} />{t.actModelUsage}</h4><p>{formatTokens(modelTotal, zh)} tokens</p></div></header>
        <div className="model-usage">
          <div className="model-pie" role="img" aria-label="模型 token 占比饼图">
            <ResponsiveContainer width="100%" height="100%" minWidth={0} initialDimension={{ width: 220, height: 220 }}>
              <PieChart>
                <Pie data={data.models} dataKey="tokens" nameKey="model" innerRadius={62} outerRadius={95} strokeWidth={0} isAnimationActive={false}>
                  {data.models.map((entry, index) => <Cell key={entry.model} fill={MODEL_COLORS[index % MODEL_COLORS.length]} />)}
                </Pie>
                <ChartTooltip content={({ active, payload }) => active && payload?.length
                  ? <div className="activity-tooltip"><strong>{payload[0].name}</strong><p>{Number(payload[0].value).toLocaleString(zh ? 'zh-CN' : 'en-US')} tokens · {modelTotal > 0 ? `${Math.round(Number(payload[0].value) / modelTotal * 100)}%` : '—'}</p></div> : null} />
              </PieChart>
            </ResponsiveContainer>
          </div>
          <ul className="model-legend">
            {data.models.map((entry, index) => <li key={entry.model}>
              <i style={{ background: MODEL_COLORS[index % MODEL_COLORS.length] }} />
              <span className="model-legend-name">{entry.model}</span>
              <span className="model-legend-value">{formatTokens(entry.tokens, zh)} · {modelTotal > 0 ? `${Math.round(entry.tokens / modelTotal * 100)}%` : '—'} · {entry.runs} {t.actRuns}</span>
            </li>)}
          </ul>
        </div>
      </section>}
    </div>
  </section>;
}
