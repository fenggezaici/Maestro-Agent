import type { QuestionAnswer, UserQuestion } from '../api/types';

export interface UserQuestionPayload {
  requestId?: string;
  questions: UserQuestion[];
  answers?: QuestionAnswer[];
}

/// Persisted ask_user detail is JSON; live "started" rows are plain text.
export function parseUserQuestionPayload(detail: string): UserQuestionPayload | null {
  try {
    const value = JSON.parse(detail) as Partial<UserQuestionPayload>;
    return Array.isArray(value.questions) ? { requestId: value.requestId, questions: value.questions, answers: value.answers } : null;
  } catch {
    return null;
  }
}

export function questionAnswerText(question: UserQuestion, answers?: QuestionAnswer[]): string {
  const answer = answers?.find((item) => item.questionId === question.id);
  if (!answer) return '未回答';
  const parts = [...answer.selected];
  const custom = answer.custom?.trim();
  if (custom) parts.push(`其他：${custom}`);
  return parts.length ? parts.join('、') : '未回答';
}
