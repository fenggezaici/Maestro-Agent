import type { PlanCard, PlanStep, RunView, SessionDetail, Snapshot } from '../api/types';
import type { ActivityDay, WorkbenchProject } from '../components/workspace';

const before = `#pragma once

#include <string>
#include "Bridge.h"

class BridgeSession {
public:
    explicit BridgeSession(Bridge* bridge)
        : bridge_(bridge) {}

    void start(const std::string& sessionId) {
        sessionId_ = sessionId;
        running_ = true;
        bridge_->open(sessionId_);
    }

    void stop() {
        bridge_->close(sessionId_);
        running_ = false;
    }

    bool isRunning() const { return running_; }

private:
    Bridge* bridge_;
    std::string sessionId_;
    bool running_ = false;
};
`;

const after = `#pragma once

#include <memory>
#include <mutex>
#include <string>
#include "Bridge.h"

class BridgeSession {
public:
    explicit BridgeSession(std::shared_ptr<Bridge> bridge)
        : bridge_(std::move(bridge)) {}

    void start(const std::string& sessionId) {
        std::scoped_lock lock(mutex_);
        if (running_) return;

        sessionId_ = sessionId;
        bridge_->open(sessionId_);
        running_ = true;
    }

    void stop() {
        std::scoped_lock lock(mutex_);
        if (!running_) return;

        bridge_->close(sessionId_);
        running_ = false;
        sessionId_.clear();
    }

    bool isRunning() const {
        std::scoped_lock lock(mutex_);
        return running_;
    }

private:
    std::shared_ptr<Bridge> bridge_;
    mutable std::mutex mutex_;
    std::string sessionId_;
    bool running_ = false;
};
`;

export const previewPlan: PlanCard = {
  id: 204, sessionId: 101, runId: 1001, status: 'pending',
  draft: {
    title: 'Session lifecycle refactor',
    summary: '保留公共 API，分三步完善会话生命周期。静态示例。',
    steps: [
      '使用共享所有权管理 Bridge，避免悬空引用。',
      '让 start / stop 幂等，并保护并发状态转换。',
      '补充生命周期回归测试，再审查完整 Diff。',
    ],
    diffs: [
      { path: 'src/bridge/BridgeSession.h', before, after },
      { path: 'tests/BridgeSession.test.cpp', before: '#include "BridgeSession.h"\n\nTEST(BridgeSession, starts) {\n    EXPECT_TRUE(session.isRunning());\n}\n', after: '#include "BridgeSession.h"\n\nTEST(BridgeSession, stop_is_idempotent) {\n    session.start("test-session");\n    session.stop();\n    session.stop();\n    EXPECT_FALSE(session.isRunning());\n}\n\nTEST(BridgeSession, start_is_idempotent) {\n    session.start("test-session");\n    session.start("test-session");\n    EXPECT_EQ(bridge.openCount(), 1);\n}\n' },
    ],
  },
};

const mainRun: RunView = {
  id: 1001, sessionId: 101, mode: 'code', status: 'completed', error: null,
  prompt: '重构 BridgeSession 的生命周期管理，保留现有 API。\n让重复 stop 安全，并修复并发状态切换问题，先提交修改计划。',
  output: '建议保留公共 API，收敛所有权与状态边界。示例 Diff 为 start / stop 增加幂等保护，并统一并发状态访问。下面是建议的实现计划。',
};

export const previewSnapshot: Snapshot = { ready: false, mode: 'code', activeRun: null, lastRun: null, sequence: 0 };

/// Static task-tracker sample for the browser-only visual preview.
export const previewTaskPlan: PlanStep[] = [
  { id: '1', description: '梳理 BridgeSession 的现有调用方与状态边界', status: 'completed' },
  { id: '2', description: '重构所有权与并发访问，保留公共 API', status: 'in_progress' },
  { id: '3', description: '补充重复 stop 与并发切换的回归测试', status: 'pending' },
];

export interface PreviewProject extends Omit<WorkbenchProject, 'sessions'> { sessions: SessionDetail[] }

const levels = [0, 1, 2, 0, 1, 3, 1, 2, 2, 0, 4, 2, 1, 3, 0, 1, 2, 3, 2, 1, 0, 2, 3, 4, 2, 3, 1, 2, 1, 2, 4, 3, 2, 1, 3] as const;
const sampleActivity: ActivityDay[] = levels.map((level, index) => ({
  date: new Date(Date.UTC(2026, 9, 5) + index * 86400000).toISOString().slice(0, 10), level,
}));

function exampleSession(id: number, title: string, projectId: number): SessionDetail {
  return { session: { id, title, updatedAt: 1, runCount: 1, projectId, archived: false }, plans: [],
    runs: [{ ...mainRun, id: id * 10, sessionId: id, prompt: title, output: '此会话属于另一个预览项目。所有内容为本地 Mock，未连接 Rust 或模型服务。' }],
  };
}

export const previewProjects: PreviewProject[] = [
  { id: '1', name: 'Maestro-Agent', activity: sampleActivity, today: { changedLines: 120, completedPlans: 3 }, sessions: [
    { session: { id: 101, title: 'Refactor session lifecycle', updatedAt: 1, runCount: 1, projectId: 1, archived: false }, runs: [mainRun], plans: [previewPlan] },
    exampleSession(102, 'Improve event dispatch pipeline', 1),
  ] },
  { id: '2', name: 'Agent Tools', activity: sampleActivity, today: { changedLines: 37, completedPlans: 1 }, sessions: [
    exampleSession(103, 'Review database migrations', 2), exampleSession(104, 'Add integration test coverage', 2),
  ] },
  { id: '3', name: 'Website', activity: sampleActivity, today: { changedLines: 0, completedPlans: 0 }, sessions: [
    exampleSession(105, 'Polish landing page', 3),
  ] },
];

export const previewSessions = previewProjects.flatMap((project) => project.sessions.map((detail) => detail.session));

export function previewDetail(id: number | null, projects = previewProjects): SessionDetail | null {
  return projects.flatMap((project) => project.sessions).find((detail) => detail.session.id === id) ?? null;
}

