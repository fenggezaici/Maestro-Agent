import { useEffect } from 'react';
import { useKernel } from './hooks/useKernel';
import Workbench from './components/Workbench';
import { migrateLegacyCredentials } from './api/credentials';
import { useKernelStore } from './stores/useKernelStore';

export default function App() {
  const controller = useKernel();
  // One-time upgrade: plaintext keys move to the OS-protected store. A failure
  // keeps the old data and surfaces a recoverable message instead of dropping it.
  useEffect(() => {
    void migrateLegacyCredentials().then((notice) => {
      if (notice) useKernelStore.setState({ error: notice });
    });
  }, []);
  return <Workbench controller={controller} />;
}
