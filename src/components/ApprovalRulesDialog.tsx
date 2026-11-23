import { useEffect, useState } from 'react';
import { Loader2, ShieldCheck, Trash2 } from 'lucide-react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';
import { kernelApi } from '../api/kernel';
import type { ApprovalRuleView } from '../api/types';

const sameRule = (left: ApprovalRuleView, right: ApprovalRuleView) =>
  left.tool === right.tool && left.pattern === right.pattern && left.prefix === right.prefix;

/// Session-scoped "always allow" rules, shown so a wide rule is never invisible.
/// Addressed by run id: the backend resolves the session and rejects unknown ids.
export default function ApprovalRulesDialog({ runId, open, onOpenChange }: {
  runId: number; open: boolean; onOpenChange: (open: boolean) => void;
}) {
  const [rules, setRules] = useState<ApprovalRuleView[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    kernelApi.listApprovalRules(runId)
      .then((next) => { if (!cancelled) setRules(next); })
      .catch((reason: unknown) => { if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [open, runId]);

  const revoke = (rule: ApprovalRuleView) => {
    setError(null);
    kernelApi.revokeApprovalRule(runId, rule)
      .then(() => setRules((current) => current.filter((item) => !sameRule(item, rule))))
      .catch((reason: unknown) => setError(reason instanceof Error ? reason.message : String(reason)));
  };

  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent variant="form" className="confirm-dialog">
      <DialogTitle className="confirm-dialog-title"><ShieldCheck size={18} />本会话授权规则</DialogTitle>
      <DialogDescription className="confirm-dialog-description">
        这些规则只在本会话内免除审批；切到「计划」模式时不生效——每项副作用调用仍需逐项批准。撤销后相同调用会重新请求批准。
      </DialogDescription>
      <div className="approval-rules-list">
        {loading ? <p><Loader2 size={13} className="spin" />正在读取规则…</p>
          : rules.length === 0 ? <p>当前会话还没有「始终允许」规则。</p>
          : rules.map((rule) => <div className="approval-rule-row" key={`${rule.tool}:${rule.pattern}:${rule.prefix}`}>
            <div>
              <strong>{rule.tool === '*' ? '本会话全部放行' : rule.tool}</strong>
              <code>{rule.tool === '*' ? '所有需要审批的调用' : rule.prefix ? `${rule.pattern} *` : rule.pattern}</code>
              <em>{rule.tool === '*'
                ? '仅本会话、不持久化；计划模式下不生效'
                : rule.prefix ? '前缀：同前缀的其他命令也会免审' : '精确匹配'}</em>
            </div>
            <button type="button" className="button-ghost" aria-label={`撤销规则 ${rule.tool} ${rule.pattern}`} onClick={() => revoke(rule)}>
              <Trash2 size={12} />撤销
            </button>
          </div>)}
        {error && <p role="alert" className="settings-form-error">{error}</p>}
      </div>
    </DialogContent>
  </Dialog>;
}
