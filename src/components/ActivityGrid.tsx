import * as Tooltip from '@radix-ui/react-tooltip';
import { useState, type CSSProperties } from 'react';
import { useT } from '../i18n';
import { aggregateHeatmap, heatmapWeeks, type DashboardDay } from './activityMock';

// 与后端 level 分级同一口径（按行数）。
const levelFor = (lines: number) => lines === 0 ? 0 : lines < 45 ? 1 : lines < 75 ? 2 : lines < 100 ? 3 : 4;

export default function ActivityGrid({ days, live = false }: { days: DashboardDay[]; live?: boolean }) {
  const t = useT();
  const [mode, setMode] = useState<'daily' | 'weekly' | 'cumulative'>('daily');
  const aggregated = aggregateHeatmap(days, mode);
  const weeks = heatmapWeeks(aggregated);
  const monthName = (date: string) => new Intl.DateTimeFormat(t.locale, { month: 'short', timeZone: 'UTC' }).format(new Date(`${date}T00:00:00Z`));
  const modes = [
    { id: 'daily' as const, label: t.actDaily },
    { id: 'weekly' as const, label: t.actWeekly },
    { id: 'cumulative' as const, label: t.actCumulative },
  ];
  return <section aria-label={t.actContributions} className="dashboard-card contributions-card">
    <header className="dashboard-section-heading"><div><h4>{t.actContributions}</h4><p>{t.actContribSubtitle}</p></div>
      <div className="segmented-control" role="group" aria-label="聚合方式">
        {modes.map((item) => <button key={item.id} type="button" className={mode === item.id ? 'active' : ''} onClick={() => setMode(item.id)}>{item.label}</button>)}
      </div>
      {!live && <span className="mock-badge">Mock</span>}</header>
    <div className="heatmap-scroll" tabIndex={0} aria-label={t.actDayAria}>
      <div className="heatmap-calendar" style={{ '--heatmap-weeks': weeks.length } as CSSProperties}>
        <div className="heatmap-months">{weeks.map((week, index) => {
          const first = week.find((day) => day?.date.endsWith('-01')) ?? (index === 0 ? week.find(Boolean) : null);
          return <span key={index}>{first ? monthName(first.date) : ''}</span>;
        })}</div>
        <div className="heatmap-body"><div className="heatmap-weekdays"><span /><span>{t.actMon}</span><span /><span>{t.actWed}</span><span /><span>{t.actFri}</span><span /></div>
          <div className="heatmap-weeks">{weeks.map((week, index) => <div key={index} className="heatmap-week">{week.map((day, row) => day
            ? <Tooltip.Root key={day.date}><Tooltip.Trigger asChild><span tabIndex={0} role="img" data-heatmap-day={day.date}
              aria-label={`${day.date}：${t.actDayTooltip(day.lines, day.tokens, day.plans)}${live ? '' : '（Mock）'}`} className={`heatmap-day activity-level-${levelFor(day.lines)}`} /></Tooltip.Trigger>
              <Tooltip.Portal><Tooltip.Content className="activity-tooltip" sideOffset={8}><strong>{live ? day.date : `${day.date} / Mock`}</strong><p>{t.actDayTooltip(day.lines, day.tokens, day.plans)}</p>{!live && <p>前端示例统计，非真实工作区活动。</p>}<Tooltip.Arrow className="fill-[#292929]" /></Tooltip.Content></Tooltip.Portal>
            </Tooltip.Root>
            : <span key={`empty-${row}`} aria-hidden="true" className="heatmap-day heatmap-padding" />)}</div>)}</div>
        </div>
      </div>
    </div>
    <footer className="heatmap-caption"><span>{days[0].date} — {days.at(-1)!.date}{live ? "" : " / Mock data"}</span><span>{t.actLess}{[0, 1, 2, 3, 4].map((level) => <i key={level} className={`heatmap-day activity-level-${level}`} />)}{t.actMore}</span></footer>
  </section>;
}
