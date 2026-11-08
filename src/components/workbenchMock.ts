export interface ExplorerFile { id: string; path: string; content: string; source: 'mock' | 'proposal' | 'disk' }
export type ExtensionKind = 'mcp' | 'skills' | 'plugins';
export interface MockExtension { id: string; name: string; description: string; kind: ExtensionKind; status: 'installed' | 'available' | 'unavailable'; enabled: boolean }

export const mockExplorerFiles: ExplorerFile[] = [
  { id: 'bridge-header', path: 'src/bridge/BridgeSession.h', source: 'mock', content: '#pragma once\n\n#include <memory>\n#include <string>\n\nclass BridgeSession {\npublic:\n    void start(const std::string& id);\n    void stop();\n\nprivate:\n    std::string sessionId_;\n    bool running_ = false;\n};\n' },
  { id: 'bridge-source', path: 'src/bridge/BridgeSession.cpp', source: 'mock', content: '#include "BridgeSession.h"\n\nvoid BridgeSession::start(const std::string& id) {\n    if (running_) return;\n    sessionId_ = id;\n    running_ = true;\n}\n\nvoid BridgeSession::stop() {\n    if (!running_) return;\n    running_ = false;\n    sessionId_.clear();\n}\n' },
  { id: 'app', path: 'src/App.tsx', source: 'mock', content: 'export default function App() {\n  return <Workbench mode="plan" />;\n}\n' },
  { id: 'test', path: 'tests/BridgeSession.test.cpp', source: 'mock', content: 'TEST(BridgeSession, stop_is_idempotent) {\n    session.stop();\n    session.stop();\n    EXPECT_FALSE(session.isRunning());\n}\n' },
  { id: 'readme', path: 'README.md', source: 'mock', content: '# Maestro-Agent\n\nThis is a virtual file preview.\nNo filesystem access or edits are performed.\n' },
];

export const mockExtensions: MockExtension[] = [
  { id: 'filesystem', name: 'Filesystem MCP', kind: 'mcp', description: 'Project context / virtual connection', status: 'installed', enabled: true },
  { id: 'git', name: 'Git MCP', kind: 'mcp', description: 'Repository insights / virtual connection', status: 'installed', enabled: false },
  { id: 'browser', name: 'Browser MCP', kind: 'mcp', description: 'Browser automation / unavailable in this demo', status: 'unavailable', enabled: false },
  { id: 'review', name: 'Code Review', kind: 'skills', description: 'Review conventions and quality checks', status: 'installed', enabled: true },
  { id: 'tests', name: 'Test Writer', kind: 'skills', description: 'Test planning and coverage suggestions', status: 'available', enabled: false },
  { id: 'formatter', name: 'Formatter', kind: 'plugins', description: 'Formatting integration / UI sample', status: 'installed', enabled: false },
  { id: 'diagnostics', name: 'Diagnostics', kind: 'plugins', description: 'Inline diagnostic integration / UI sample', status: 'available', enabled: false },
];

export function changeMockExtension(items: MockExtension[], id: string, action: 'toggle' | 'install' | 'remove'): MockExtension[] {
  return items.map((item) => {
    if (item.id !== id || item.status === 'unavailable') return item;
    if (action === 'install' && item.status === 'available') return { ...item, status: 'installed', enabled: false };
    if (action === 'remove' && item.status === 'installed') return { ...item, status: 'available', enabled: false };
    if (action === 'toggle' && item.status === 'installed') return { ...item, enabled: !item.enabled };
    return item;
  });
}
