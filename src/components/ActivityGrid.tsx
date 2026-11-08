import * as Tooltip from '@radix-ui/react-tooltip';
import type { CSSProperties } from 'react';
import { heatmapWeeks, type DashboardDay } from './activityMock';

export default function ActivityGrid({ days }: { days: DashboardDay[] }) {
  const weeks = heatmapWeeks(days);
  const monthName = (date: string) => new Intl.DateTimeFormat('en-US', { month: 'short', timeZone: 'UTC' }).format(new Date(`${date}T00:00:00Z`));
  return <section aria-label="六个月活跃度" className="dashboard-card contributions-card">
    <header className="dashboard-section-heading"><div><h4>Contributions</h4><p>过去六个月的代码与计划活动</p></div><span className="mock-badge">Mock</span></header>
    <div className="heatmap-scroll" tabIndex={0} aria-label="活跃度日历，可横向滚动">
      <div className="heatmap-calendar" style={{ '--heatmap-weeks': weeks.length } as CSSProperties}>
        <div className="heatmap-months">{weeks.map((week, index) => {
          const first = week.find((day) => day?.date.endsWith('-01')) ?? (index === 0 ? week.find(Boolean) : null);
          return <span key={index}>{first ? monthName(first.date) : ''}</span>;
        })}</div>
        <div className="heatmap-body"><div className="heatmap-weekdays"><span /><span>Mon</span><span /><span>Wed</span><span /><span>Fri</span><span /></div>
          <div className="heatmap-weeks">{weeks.map((week, index) => <div key={index} className="heatmap-week">{week.map((day, row) => day
            ? <Tooltip.Root key={day.date}><Tooltip.Trigger asChild><span tabIndex={0} role="img" data-heatmap-day={day.date}
              aria-label={`${day.date}：${day.lines} 行，${day.tokens} tokens，${day.plans} 个计划（Mock）`} className={`heatmap-day activity-level-${day.level}`} /></Tooltip.Trigger>
              <Tooltip.Portal><Tooltip.Content className="activity-tooltip" sideOffset={8}><strong>{day.date} / Mock</strong><p>修改 {day.lines} 行代码 · {day.tokens.toLocaleString('en-US')} tokens · {day.plans} 个计划</p><p>前端示例统计，非真实工作区活动。</p><Tooltip.Arrow className="fill-[#292929]" /></Tooltip.Content></Tooltip.Portal>
            </Tooltip.Root>
            : <span key={`empty-${row}`} aria-hidden="true" className="heatmap-day heatmap-padding" />)}</div>)}</div>
        </div>
      </div>
    </div>
    <footer className="heatmap-caption"><span>{days[0].date} — {days.at(-1)!.date} / Mock data</span><span>Less{[0, 1, 2, 3, 4].map((level) => <i key={level} className={`heatmap-day activity-level-${level}`} />)}More</span></footer>
  </section>;
}

