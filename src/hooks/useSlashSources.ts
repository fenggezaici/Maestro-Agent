import { useCallback, useEffect, useState } from 'react';
import { extensionsApi, type CommandView, type SkillView } from '../api/extensions';

/// Skills and command files for the composer's slash palette. Reloaded when the
/// project changes and whenever the palette opens.
export default function useSlashSources(enabled: boolean, workspaceKey: string | number) {
  const [skills, setSkills] = useState<SkillView[]>([]);
  const [commands, setCommands] = useState<CommandView[]>([]);
  const refresh = useCallback(async () => {
    if (!enabled) return;
    try {
      const [nextSkills, nextCommands] = await Promise.all([
        extensionsApi.scanSkills(),
        extensionsApi.scanCommands(),
      ]);
      setSkills(nextSkills);
      setCommands(nextCommands);
    } catch {
      // Keep the previous list; an empty palette is better than a broken composer.
    }
  }, [enabled, workspaceKey]);
  useEffect(() => { void refresh(); }, [refresh]);
  return { skills, commands, refresh };
}
