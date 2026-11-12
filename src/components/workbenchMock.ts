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

// Preview-mode controller: functional local state, zero backend calls.
import { useState } from 'react';
import type { ExtensionsController } from '../hooks/useExtensions';
import type { McpServerView, RecommendedItem, SkillView } from '../api/extensions';

const previewRecommended: RecommendedItem[] = [
  { id: 'superpowers', kind: 'skill', name: 'Superpowers', description: '社区热门技能合集：TDD、系统化调试、分支工作流。', sourceUrl: 'https://github.com/obra/superpowers', installPayload: 'https://github.com/obra/superpowers' },
  { id: 'filesystem', kind: 'mcp', name: 'FileSystem', description: '官方文件系统服务：受控目录内的文件读写与目录遍历。', sourceUrl: 'https://github.com/modelcontextprotocol/servers', installPayload: '{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem"]}' },
];

export function usePreviewExtensions(): ExtensionsController {
  const [skills, setSkills] = useState<SkillView[]>([
    { name: 'code-review', description: 'Review conventions and quality checks', source: 'global', path: '~/.maestro/skills/code-review/SKILL.md' },
    { name: 'pdf-tools', description: '项目级技能示例', source: 'project', path: '<workspace>/.maestro/skills/pdf-tools/SKILL.md' },
  ]);
  const [servers, setServers] = useState<McpServerView[]>([
    { name: 'filesystem', command: 'npx', args: ['-y', 'server-fs'], disabled: false, trust: false, running: true, toolCount: 8, error: null },
    { name: 'github', command: 'npx', args: ['-y', 'server-gh'], disabled: false, trust: false, running: false, toolCount: 0, error: null },
  ]);
  const [pending, setPending] = useState<string | null>(null);
  const noop = async () => null;
  return {
    skills, servers,
    recommendedSkills: previewRecommended.filter((i) => i.kind === 'skill'),
    recommendedMcps: previewRecommended.filter((i) => i.kind === 'mcp'),
    mcpConfigText: '{\n  "mcpServers": {\n    "filesystem": { "command": "npx", "args": ["-y", "server-fs"] }\n  }\n}',
    loading: false, pending,
    importSummary: null, real: false,
    installErrors: {}, isInstalled: () => false,
    refresh: async () => {},
    createSkill: async (name, description) => { setSkills((items) => [...items, { name, description, source: 'global', path: `~/.maestro/skills/${name}/SKILL.md` }]); return null; },
    importSkill: noop, importSkillGithub: noop,
    addMcpServer: async (name, command, args) => { setServers((items) => [...items, { name, command, args, disabled: false, trust: false, running: false, toolCount: 0, error: null }]); return null; },
    setMcpTrust: async (server, trust) => { setServers((items) => items.map((item) => item.name === server.name ? { ...item, trust } : item)); },
    importMcpJson: noop,
    toggleMcpServer: async (server, enable) => {
      setPending(`mcp-${server.name}`);
      setServers((items) => items.map((item) => item.name === server.name ? { ...item, running: enable, toolCount: enable ? 8 : 0 } : item));
      setPending(null);
    },
    removeMcpServer: async (name) => { setServers((items) => items.filter((item) => item.name !== name)); return null; },
    installRecommended: noop,
  };
}
