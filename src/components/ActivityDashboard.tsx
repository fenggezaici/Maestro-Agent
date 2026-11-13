import { ArrowLeft, CheckCheck, Coins, FileCode2, Sparkles, TrendingUp } from 'lucide-react';
import { useT } from '../i18n';
import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from 'recharts';
import ActivityGrid from './ActivityGrid';
import type { DashboardData } from './activityMock';

export default function ActivityDashboard({ data, projectName, onBack, loading = false, error = null }: {
  data: DashboardData; projectName: string; onBack: () => void; loading?: boolean; error?: string | null;
}) {
  const t = useT();
  const live = data.source === 'live';
  const metrics = [
    { label: 'Total Lines', value: data.totalLines.toLocaleString('en-US'), detail: 'Lines changed', icon: FileCode2 },
    { label: 'Plans Approved', value: String(data.approvedPlans), detail: 'Plans reviewed', icon: CheckCheck },
    { label: 'Tokens Used', value: `${(data.totalTokens / 1000000).toFixed(1)}M`, detail: 'Model tokens', icon: Sparkles },
    { label: 'Cost', value: data.cost === null ? '—' : `$${data.cost.toFixed(2)}`, detail: 'Estimated usage', icon: Coins },
  ];
  const trendTotal = data.trend.reduce((total, day) => total + day.tokens, 0);
  return <section aria-label="Activity Dashboard" className="dashboard-scroll">
    <div className="dashboard-inner">
      <button type="button" className="dashboard-back" onClick={onBack}><ArrowLeft size={13} />{t.backToChat}</button>
      <div className="dashboard-title"><div><p className="eyebrow">{t.workspaceInsights}</p><h3>{t.activityDashboard}</h3><p>{projectName} / 过去六个月的工作节奏</p></div>{!live && <span className="mock-badge">Mock data</span>}</div>
      {loading && <p className="dashboard-disclaimer">正在读取本地活动数据…</p>}
      {error && <p role="alert" className="dashboard-disclaimer">活动数据加载失败：{error}</p>}
      {!live && <p className="dashboard-disclaimer">所有指标均为前端 Mock，不代表真实代码修改、模型用量或账单。</p>}
      {live && data.totalLines === 0 && <p className="dashboard-disclaimer">修改行数仅统计接入后的文件变更；历史会话没有 before/after 记录。</p>}
      <div className="dashboard-metrics" aria-label="核心指标">
        {metrics.map(({ label, value, detail, icon: Icon }) => <article key={label} aria-label={live ? label : `${label}（Mock）`} className="metric-card">
          <div className="metric-heading"><Icon size={14} strokeWidth={1.5} />{!live && <span className="mock-badge">Mock</span>}</div>
          <p>{label}</p><strong>{value}</strong><small>{detail}</small>
        </article>)}
      </div>
      <ActivityGrid days={data.days} live={live} />
      <section aria-label="30 日 Token 趋势" className="dashboard-card trend-card">
        <header className="dashboard-section-heading"><div><h4><TrendingUp size={15} />Token usage</h4><p>Last 30 days / {trendTotal.toLocaleString('en-US')} tokens</p></div>{!live && <span className="mock-badge">Mock</span>}</header>
        <div className="trend-chart" role="img" aria-label={live ? "过去 30 天 Token 消耗折线图" : "过去 30 天 Token 消耗折线图（Mock）"}>
          <ResponsiveContainer width="100%" height="100%" minWidth={0} initialDimension={{ width: 520, height: 205 }}>
            <LineChart data={data.trend} margin={{ top: 15, right: 8, bottom: 0, left: -14 }}>
              <CartesianGrid stroke="#ffffff08" strokeDasharray="3 5" vertical={false} />
              <XAxis dataKey="shortDate" axisLine={false} tickLine={false} minTickGap={42} tick={{ fill: '#7d7d7d', fontSize: 9 }} tickMargin={12} />
              <YAxis axisLine={false} tickLine={false} width={44} tick={{ fill: '#7d7d7d', fontSize: 9 }} tickFormatter={(value: number) => `${Math.round(value / 1000)}k`} />
              <ChartTooltip cursor={{ stroke: '#ffffff15' }} content={({ active, payload, label }) => active && payload?.length
                ? <div className="activity-tooltip"><strong>{label} / Mock</strong><p>{Number(payload[0].value).toLocaleString('en-US')} tokens</p></div> : null} />
              <Line type="monotone" dataKey="tokens" stroke="#aa97c3" strokeWidth={2} dot={false} activeDot={{ r: 4, fill: '#b8a2d1', stroke: '#242424', strokeWidth: 2 }} isAnimationActive={false} />
            </LineChart>
          </ResponsiveContainer>
        </div>
        <p className="trend-caption">{live ? "30 个每日样本 · UTC · 来自本地 run_metrics。" : "30 个每日样本 · UTC · 示例趋势，不连接用量或计费 API。"}</p>
      </section>
    </div>
  </section>;
}
