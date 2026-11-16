import { describe, expect, it } from 'vitest';
import { parseUserQuestionPayload, questionAnswerText } from './userQuestion';
import type { UserQuestion } from '../api/types';

const question: UserQuestion = { id: 'scope', header: '范围', question: '分析哪些目录？', options: [{ label: 'src' }, { label: 'tests' }] };

describe('user question helpers', () => {
  it('parses persisted ask_user payloads and ignores plain text', () => {
    const payload = parseUserQuestionPayload(JSON.stringify({ requestId: 'q-1', questions: [question], answers: [{ questionId: 'scope', selected: ['src'] }] }));
    expect(payload?.requestId).toBe('q-1');
    expect(payload?.questions).toHaveLength(1);
    expect(parseUserQuestionPayload('2 个问题')).toBeNull();
    expect(parseUserQuestionPayload('{')).toBeNull();
  });

  it('formats selected options and custom answers', () => {
    expect(questionAnswerText(question, [{ questionId: 'scope', selected: ['src'] }])).toBe('src');
    expect(questionAnswerText(question, [{ questionId: 'scope', selected: [], custom: '只分析 houdini/' }])).toBe('其他：只分析 houdini/');
    expect(questionAnswerText(question, [])).toBe('未回答');
  });
});
